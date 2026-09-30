use crate::dns::DnsForwarder;
use crate::paths::Paths;
use chrono::Local;
use std::ffi::CString;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const LAUNCH_SCRIPT: &str = r#"
app_dir=$1 shim_dir=$2; shift 2
extra_args=()
for kv in "$@"; do
    if [[ "$kv" == roblox:* ]] || [[ "$kv" == roblox://* ]] || [[ "$kv" == roblox-player:* ]]; then
        export MACOBLOX_LAUNCH_URL="$kv"
        extra_args+=("$kv")
    elif [[ "$kv" == --* ]]; then
        extra_args+=("$kv")
    else
        export "$kv"
    fi
done
cd "$app_dir" || exit 1
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="$shim_dir/libMacOBloxShims.dylib"
export DYLD_LIBRARY_PATH="$shim_dir:$app_dir"
exec ./RobloxPlayer "${extra_args[@]}"
"#;

pub struct HostAudio {
    pub fifo_path: PathBuf,
    #[allow(dead_code)]
    pub keep_file: fs::File,
    pub player: Child,
}

impl HostAudio {
    pub fn is_pipewire_active() -> bool {
        if std::env::var_os("PIPEWIRE_REMOTE").is_some() {
            return true;
        }
        let uid = unsafe { libc::getuid() };
        let pw_sock = std::path::PathBuf::from(format!("/run/user/{uid}/pipewire-0"));
        if pw_sock.exists() {
            return true;
        }
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            if std::path::Path::new(&runtime_dir).join("pipewire-0").exists() {
                return true;
            }
        }
        false
    }

    pub fn start(cache_dir: &std::path::Path) -> Option<Self> {
        let is_pw = Self::is_pipewire_active();

        // Priority depending on whether PipeWire is the active sound server
        let candidates = if is_pw {
            vec!["pw-cat", "pw-play", "pacat", "paplay"]
        } else {
            vec!["pacat", "paplay", "pw-cat", "pw-play"]
        };

        let mut chosen_bin = None;
        for bin in candidates {
            if find_binary(bin).is_some()
                || Command::new(bin)
                    .arg("--version")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            {
                chosen_bin = Some(bin);
                break;
            }
        }

        let player_bin = chosen_bin?;
        let is_pipewire = player_bin == "pw-cat" || player_bin == "pw-play";

        fs::create_dir_all(cache_dir).ok()?;
        let fifo_path = cache_dir.join(format!("audio-{}.fifo", std::process::id()));
        if fifo_path.exists() {
            let _ = fs::remove_file(&fifo_path);
        }

        let c_fifo = CString::new(fifo_path.to_str()?).ok()?;
        unsafe {
            if libc::mkfifo(c_fifo.as_ptr(), 0o600) != 0 {
                return None;
            }
        }

        // Open read-write non-blocking so the writer stays open continuously
        // without waiting for an external reader (opening O_WRONLY | O_NONBLOCK returns ENXIO on Linux).
        let keep_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
            .ok()?;

        use std::os::unix::io::AsRawFd;
        unsafe {
            // Buffer size ~64KB (~185ms headroom) prevents underruns while keeping latency low
            let _ = libc::fcntl(keep_file.as_raw_fd(), libc::F_SETPIPE_SZ, 65536);
        }

        let mut cmd = Command::new(player_bin);
        if is_pipewire {
            cmd.args([
                "--playback", "--raw", "--format", "f32", "--rate", "44100",
                "--channels", "2", "--latency", "40ms", "--media-role", "Game",
                "-P", "{ application.name = \"Roblox\" application.process.binary = \"crabblox\" media.name = \"Roblox (Crabblox)\" node.name = \"Roblox\" node.latency = 1024/44100 }",
                fifo_path.to_str()?,
            ]);
            cmd.env("PIPEWIRE_LATENCY", "1024/44100");
        } else {
            cmd.args([
                "--playback", "--raw", "--format=float32le", "--rate=44100",
                "--channels=2", "--latency-msec=40", "--client-name=Roblox",
                "--stream-name=Roblox (Crabblox)", "--property=media.role=game",
                fifo_path.to_str()?,
            ]);
            cmd.env("PULSE_LATENCY_MSEC", "40");
        }

        let player = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;

        Some(Self {
            fifo_path,
            keep_file,
            player,
        })
    }
}

impl Drop for HostAudio {
    fn drop(&mut self) {
        let _ = self.player.kill();
        let _ = self.player.wait();
        let _ = fs::remove_file(&self.fifo_path);
    }
}

pub struct RobloxSession {
    pub dns: DnsForwarder,
    #[allow(dead_code)]
    pub audio: Option<HostAudio>,
    pub rpc: Option<crate::discord_rpc::DiscordRpc>,
    pub log_path: PathBuf,
    pub child: Child,
    pub diagnostics_performed: bool,
}

#[allow(dead_code)]
impl RobloxSession {
    pub fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        if !status.success() && !self.diagnostics_performed {
            scan_crash_diagnostics_with_status(&self.log_path, Some(&status));
            self.diagnostics_performed = true;
        }
        Ok(status)
    }

    pub async fn wait_async(&mut self) -> std::io::Result<std::process::ExitStatus> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                if !status.success() && !self.diagnostics_performed {
                    scan_crash_diagnostics_with_status(&self.log_path, Some(&status));
                    self.diagnostics_performed = true;
                }
                return Ok(status);
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    pub fn scan_diagnostics(&mut self) -> Vec<CrashDiagnosis> {
        if !self.diagnostics_performed {
            let res = scan_crash_diagnostics_with_status(&self.log_path, None);
            self.diagnostics_performed = true;
            res
        } else {
            Vec::new()
        }
    }
}

impl Drop for RobloxSession {
    fn drop(&mut self) {
        if let Some(ref r) = self.rpc {
            r.stop();
        }
        self.dns.stop();
        let _ = self.child.kill();
        let _ = self.child.wait();
        stop_roblox();
        if !self.diagnostics_performed {
            if let Ok(Some(status)) = self.child.try_wait() {
                if !status.success() {
                    scan_crash_diagnostics_with_status(&self.log_path, Some(&status));
                    self.diagnostics_performed = true;
                }
            }
        }
    }
}

pub fn ensure_shim(paths: &Paths) -> anyhow::Result<()> {
    let shim = paths.shim_dylib();
    let host_frameworks = paths.project_dir.join("build/frameworks/CoreML.framework").exists()
        || paths.project_dir.join("prebuilt/frameworks/CoreML.framework").exists()
        || paths.data_dir.join("build/frameworks/CoreML.framework").exists()
        || paths.data_dir.join("prebuilt/frameworks/CoreML.framework").exists();
    let needs_build = !shim.exists() || !host_frameworks;

    if needs_build {
        println!("Building libMacOBloxShims.dylib and stub frameworks...");
        let script = paths.project_dir.join("build_debug_shim.sh");
        if script.exists() {
            let status = Command::new("bash")
                .arg(&script)
                .current_dir(&paths.project_dir)
                .status()?;
            if !status.success() {
                anyhow::bail!("Failed to build libMacOBloxShims.dylib");
            }
        } else {
            anyhow::bail!("build_debug_shim.sh not found at {:?}", script);
        }
    }

    // Ensure missing frameworks in Darling prefix
    let frameworks = ["CoreML", "CoreHaptics", "DeviceCheck"];
    let darling_frameworks = paths.darling_prefix.join("System/Library/Frameworks");
    fs::create_dir_all(&darling_frameworks)?;

    for name in frameworks {
        let dest = darling_frameworks.join(format!("{name}.framework"));
        let candidates = [
            paths.project_dir.join(format!("build/frameworks/{name}.framework")),
            paths.project_dir.join(format!("prebuilt/frameworks/{name}.framework")),
            paths.data_dir.join(format!("build/frameworks/{name}.framework")),
            paths.data_dir.join(format!("prebuilt/frameworks/{name}.framework")),
        ];
        let src = candidates.into_iter().find(|p| p.exists());
        if let Some(src_path) = src {
            if !dest.exists() {
                let _ = Command::new("cp")
                    .args(["-r", src_path.to_str().unwrap(), dest.to_str().unwrap()])
                    .status();
            }
        }
    }

    // Ensure Darling container has working resolv.conf pointing to local DNS
    let darling_etc = paths.darling_prefix.join("etc");
    if fs::create_dir_all(&darling_etc).is_ok() {
        let resolv = darling_etc.join("resolv.conf");
        if !resolv.exists() {
            let _ = fs::write(&resolv, "nameserver 127.0.0.53\nnameserver 1.1.1.1\n");
        }
    }

    Ok(())
}

pub fn process_state(pid: i32) -> Option<char> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit(')').next()?;
    let state = after_comm.split_whitespace().next()?.chars().next()?;
    Some(state)
}

#[allow(dead_code)]
pub fn get_parent_pid(pid: i32) -> Option<i32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit(')').next()?;
    let mut parts = after_comm.split_whitespace();
    let _state = parts.next()?;
    let ppid_str = parts.next()?;
    ppid_str.parse::<i32>().ok()
}

pub fn check_disk_space(path: &std::path::Path, min_bytes: u64) -> anyhow::Result<()> {
    let path_str = path.to_str().unwrap_or("/tmp");
    let c_path = CString::new(path_str).map_err(|e| anyhow::anyhow!("Invalid path: {e}"))?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            let free_bytes = (stat.f_bavail as u64) * (stat.f_bsize as u64);
            if free_bytes < min_bytes {
                let free_mb = free_bytes >> 20;
                let min_mb = min_bytes >> 20;
                anyhow::bail!(
                    "Недостаточно свободного места на диске ({free_mb} МБ доступно, требуется минимум {min_mb} МБ)."
                );
            }
        }
    }
    Ok(())
}

pub fn raise_fd_limit() {
    unsafe {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
            let target = 65536;
            let new_cur = if rlim.rlim_max > 0 {
                target.min(rlim.rlim_max)
            } else {
                target
            };
            if new_cur > rlim.rlim_cur {
                rlim.rlim_cur = new_cur;
                let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &rlim);
            }
        }
    }
}

#[allow(dead_code)]
pub struct GpuEnvironment {
    pub is_nvidia: bool,
    pub is_amd: bool,
    pub is_intel: bool,
    pub is_hybrid: bool,
    pub env_vars: Vec<(String, String)>,
}

pub fn detect_gpu_environment() -> GpuEnvironment {
    let mut is_nvidia = false;
    let mut is_amd = false;
    let mut is_intel = false;
    let mut drm_card_count = 0;

    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("card") && !name.contains('-') {
                drm_card_count += 1;
                let uevent_path = entry.path().join("device/uevent");
                if let Ok(uevent) = fs::read_to_string(&uevent_path) {
                    let uevent_lower = uevent.to_lowercase();
                    if uevent_lower.contains("driver=nvidia") || uevent_lower.contains("vendor=0x10de") {
                        is_nvidia = true;
                    }
                    if uevent_lower.contains("driver=amdgpu") || uevent_lower.contains("driver=radeon") || uevent_lower.contains("vendor=0x1002") {
                        is_amd = true;
                    }
                    if uevent_lower.contains("driver=i915") || uevent_lower.contains("driver=xe") || uevent_lower.contains("vendor=0x8086") {
                        is_intel = true;
                    }
                }
            }
        }
    }

    if !is_nvidia && (std::path::Path::new("/proc/driver/nvidia").exists() || std::path::Path::new("/dev/nvidia0").exists()) {
        is_nvidia = true;
    }

    let is_hybrid = drm_card_count >= 2;
    let mut env_vars = Vec::new();

    if is_nvidia {
        env_vars.push(("__GL_SHADER_DISK_CACHE".into(), "1".into()));
        env_vars.push(("__GL_SHADER_DISK_CACHE_SIZE".into(), "1073741824".into()));
        if is_hybrid {
            env_vars.push(("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into()));
            env_vars.push(("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into()));
            env_vars.push(("__VK_LAYER_NV_optimus".into(), "NVIDIA_only".into()));
        }
    }

    if is_amd {
        env_vars.push(("RADV_PERFTEST".into(), "aco".into()));
        env_vars.push(("AMD_VULKAN_ICD".into(), "RADV".into()));
        env_vars.push(("mesa_glthread".into(), "true".into()));
        if is_hybrid && !is_nvidia {
            env_vars.push(("DRI_PRIME".into(), "1".into()));
        }
    }

    if is_intel {
        env_vars.push(("MESA_LOADER_DRIVER_OVERRIDE".into(), "iris".into()));
        env_vars.push(("mesa_glthread".into(), "true".into()));
    }

    // Modern Mesa performance flags: single-file disk cache prevents inode flooding & stutter,
    // 2GB shader cache prevents eviction.
    env_vars.push(("MESA_DISK_CACHE_SINGLE_FILE".into(), "1".into()));
    env_vars.push(("MESA_GLSL_CACHE_MAX_SIZE".into(), "2G".into()));
    env_vars.push(("MESA_SHADER_CACHE_MAX_SIZE".into(), "2G".into()));

    // Under Wayland / Xwayland, forcing vblank_mode=0 thrashes compositor buffer presentation,
    // causing extreme stutter and frame drops on 120Hz/144Hz displays. Only use on X11.
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok()
        || std::env::var("XDG_SESSION_TYPE").map(|v| v.eq_ignore_ascii_case("wayland")).unwrap_or(false);
    if !is_wayland {
        env_vars.push(("vblank_mode".into(), "0".into()));
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let mvk_cache_dir = home.join(".cache/crabblox/vulkan_cache");
    let _ = fs::create_dir_all(&mvk_cache_dir);
    env_vars.push(("MVK_CONFIG_RESUME_NEW_PIPELINES_IMMEDIATELY".into(), "1".into()));
    env_vars.push(("MVK_CONFIG_PREFILL_METAL_COMMAND_BUFFERS".into(), "3".into()));
    env_vars.push(("VK_PIPELINE_CACHE_ENABLE".into(), "1".into()));
    env_vars.push(("VK_PIPELINE_CACHE_PATH".into(), mvk_cache_dir.display().to_string()));

    GpuEnvironment {
        is_nvidia,
        is_amd,
        is_intel,
        is_hybrid,
        env_vars,
    }
}

pub fn clear_stale_darling(prefix: &std::path::Path) {
    let init_pid_path = prefix.join(".init.pid");
    let sock_path = prefix.join(".darlingserver.sock");

    if let Ok(content) = fs::read_to_string(&init_pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            let state = process_state(pid);
            if state.is_none() || state == Some('Z') {
                let _ = fs::remove_file(&init_pid_path);
                let _ = fs::remove_file(&sock_path);
            }
        } else {
            let _ = fs::remove_file(&init_pid_path);
            let _ = fs::remove_file(&sock_path);
        }
    } else if sock_path.exists() {
        let _ = fs::remove_file(&sock_path);
    }

    let shellspawn_sock = prefix.join("var/run/shellspawn.sock");
    if shellspawn_sock.exists() && !is_darlingserver_running(prefix) {
        let _ = fs::remove_file(&shellspawn_sock);
    }

    // Also clean up any abandoned Darling sockets in /tmp for this user
    let uid = unsafe { libc::getuid() };
    let username = std::env::var("USER").unwrap_or_default();
    let tmp_candidates = [
        format!("/tmp/darling-{}", username),
        format!("/tmp/darling-{}", uid),
        format!("/tmp/darlingserver-{}", username),
        format!("/tmp/darlingserver-{}", uid),
    ];
    for tmp_sock in tmp_candidates {
        let path = std::path::Path::new(&tmp_sock);
        if path.exists() && !is_darlingserver_running(prefix) {
            let _ = fs::remove_file(path);
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub fn darlingservers(prefix: &std::path::Path) -> Vec<i32> {
    let mut pids = Vec::new();
    let prefix_str = prefix.to_string_lossy();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() {
                if let Ok(bytes) = fs::read(entry.path().join("cmdline")) {
                    let cmd = String::from_utf8_lossy(&bytes);
                    if cmd.contains("darlingserver") && cmd.contains(&*prefix_str) {
                        if let Some(state) = process_state(pid) {
                            if state != 'Z' {
                                pids.push(pid);
                            }
                        }
                    }
                }
            }
        }
    }
    pids
}

pub fn is_darlingserver_running(prefix: &std::path::Path) -> bool {
    !darlingservers(prefix).is_empty()
}

pub fn restart_darling(prefix: &std::path::Path) {
    let _ = Command::new("darling")
        .arg("shutdown")
        .env("DPREFIX", prefix)
        .status();

    for pid in darlingservers(prefix) {
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }

    stop_roblox();

    std::thread::sleep(std::time::Duration::from_millis(500));

    clear_stale_darling(prefix);

    let _ = fs::remove_file(prefix.join(".init.pid"));
    let _ = fs::remove_file(prefix.join(".darlingserver.sock"));
    let _ = fs::remove_file(prefix.join("var/run/shellspawn.sock"));
}

pub fn roblox_pids() -> Vec<i32> {
    let mut pids = Vec::new();
    let my_pid = std::process::id() as i32;
    let my_uid = unsafe { libc::getuid() };
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() != my_uid {
                    continue;
                }
            }
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<i32>() {
                if pid == my_pid {
                    continue;
                }
                let cmdline = entry.path().join("cmdline");
                if let Ok(bytes) = fs::read(cmdline) {
                    let cmd = String::from_utf8_lossy(&bytes);
                    if cmd.contains("RobloxPlayer")
                        && !cmd.contains("RobloxCrashHandler")
                        && !cmd.contains("crabblox")
                    {
                        pids.push(pid);
                    }
                }
            }
        }
    }
    pids
}

pub fn all_roblox_pids() -> Vec<i32> {
    let mut pids = Vec::new();
    let my_pid = std::process::id() as i32;
    let my_uid = unsafe { libc::getuid() };
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                use std::os::unix::fs::MetadataExt;
                if meta.uid() != my_uid {
                    continue;
                }
            }
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<i32>() {
                if pid == my_pid {
                    continue;
                }
                let cmdline = entry.path().join("cmdline");
                if let Ok(bytes) = fs::read(cmdline) {
                    let cmd = String::from_utf8_lossy(&bytes);
                    if (cmd.contains("RobloxPlayer") || cmd.contains("RobloxCrashHandler"))
                        && !cmd.contains("crabblox")
                    {
                        pids.push(pid);
                    }
                }
            }
        }
    }
    pids
}

pub fn stop_roblox() {
    let pids = all_roblox_pids();
    for pid in &pids {
        unsafe {
            libc::kill(*pid, libc::SIGTERM);
        }
    }
    if !pids.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(300));
        for pid in all_roblox_pids() {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

pub fn host_vram_bytes() -> u64 {
    let mut best = 0u64;
    if let Ok(entries) = fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let path = entry.path().join("device/mem_info_vram_total");
            if let Ok(s) = fs::read_to_string(path) {
                if let Ok(val) = s.trim().parse::<u64>() {
                    best = best.max(val);
                }
            }
        }
    }
    if best >= (64 << 20) {
        best
    } else {
        4 * 1024 * 1024 * 1024
    }
}

pub async fn launch(paths: &Paths) -> anyhow::Result<RobloxSession> {
    ensure_shim(paths)?;

    // Raise file descriptor limits (ulimit -n) to prevent "Too many open files"
    raise_fd_limit();

    // Check disk space before launching: ensure at least 500 MB is available in Darling prefix
    check_disk_space(&paths.darling_prefix, 500 * 1024 * 1024)?;

    let app_bundle = paths.app_bundle();
    let binary = app_bundle.join("Contents/MacOS/RobloxPlayer");
    if !binary.exists() {
        println!("RobloxPlayer not found, downloading latest version...");
        let (_human_ver, upload) = crate::updater::latest_version()
            .map_err(|e| anyhow::anyhow!("Не удалось получить информацию о последней версии Roblox: {e}"))?;
        crate::updater::update_roblox(paths, &upload, |frac, msg| {
            println!("[{:>3}%] {}", (frac * 100.0) as u32, msg);
        })
        .map_err(|e| anyhow::anyhow!("Не удалось скачать и установить Roblox: {e}"))?;
    }

    // Ensure FastFlags required for Darling compatibility (disable CookieProtocol for login sync)
    if let Err(e) = crate::fast_flags::FastFlags::ensure_compatibility_flags(paths) {
        eprintln!("Warning: failed to ensure compatibility FastFlags: {e}");
    }

    // Ensure session cookie is present and restored into Darling prefix
    let _ = crate::auth::ensure_session_restored(paths);

    // Clean any stale Darling mounts or state
    clear_stale_darling(&paths.darling_prefix);

    // Warm up darlingserver if not running
    if !is_darlingserver_running(&paths.darling_prefix) {
        let warmup = Command::new("darling")
            .args(["shell", "/bin/true"])
            .env("DPREFIX", &paths.darling_prefix)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if let Ok(status) = warmup {
            if !status.success() {
                restart_darling(&paths.darling_prefix);
                let _ = Command::new("darling")
                    .args(["shell", "/bin/true"])
                    .env("DPREFIX", &paths.darling_prefix)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
        clear_stale_darling(&paths.darling_prefix);
    }

    let dns = DnsForwarder::start().await?;
    let dns_address = format!("127.0.0.1:{}", dns.port);

    let audio = HostAudio::start(&paths.cache_dir);

    let logs_dir = paths.logs_dir();
    fs::create_dir_all(&logs_dir)?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S");
    let log_path = logs_dir.join(format!("launch-{timestamp}.log"));
    let log_file = fs::File::create(&log_path)?;

    let shim_parent = paths.shim_dylib().parent().map(|p| p.to_path_buf()).unwrap_or_else(|| paths.project_dir.clone());
    fs::create_dir_all(&paths.data_dir)?;
    fs::create_dir_all(&shim_parent)?;

    let app_macos = paths.app_bundle().join("Contents/MacOS");
    if !app_macos.exists() {
        anyhow::bail!("RobloxPlayer.app/Contents/MacOS does not exist at {:?}", app_macos);
    }
    let darling_app_dir = format!("/Volumes/SystemRoot{}", app_macos.canonicalize()?.display());
    let darling_shim_dir = format!("/Volumes/SystemRoot{}", shim_parent.canonicalize()?.display());

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let shader_cache = home.join(".cache/mesa_shader_cache");
    let _ = fs::create_dir_all(&shader_cache);

    let vram = host_vram_bytes();
    let gpu_env = detect_gpu_environment();

    let mut args: Vec<String> = vec![
        "shell".into(),
        "/bin/bash".into(),
        "-c".into(),
        LAUNCH_SCRIPT.into(),
        "macoblox".into(),
        darling_app_dir,
        darling_shim_dir,
        format!("MACOBLOX_DNS={}", dns_address),
        format!("MACOBLOX_VRAM_BYTES={}", vram),
        format!("MESA_SHADER_CACHE_DIR={}", shader_cache.display()),
        "__GL_SHADER_DISK_CACHE=1".into(),
        format!("__GL_SHADER_DISK_CACHE_PATH={}", home.join(".cache").display()),
        "MACOBLOX_MOUSE_SENSITIVITY=1.00".into(),
        "PIPEWIRE_LATENCY=256/44100".into(),
        "PULSE_LATENCY_MSEC=50".into(),
    ];

    for (k, v) in &gpu_env.env_vars {
        args.push(format!("{}={}", k, v));
    }

    if gpu_env.is_nvidia {
        println!("GPU: NVIDIA (PRIME offload & 1GB shader cache active)");
    } else if gpu_env.is_amd {
        println!("GPU: AMD (RADV ACO shader compiler active)");
    } else if gpu_env.is_intel {
        println!("GPU: Intel Iris (OpenGL 4.6, 2GB single-file shader cache active)");
    }

    let icon_file = paths.cache_dir.join("icon.argb");
    if icon_file.exists() {
        args.push(format!("MACOBLOX_ICON_ARGB=/Volumes/SystemRoot{}", icon_file.display()));
    }

    if let Some(ref a) = audio {
        args.push(format!("MACOBLOX_AUDIO_FIFO=/Volumes/SystemRoot{}", a.fifo_path.display()));
    } else {
        args.push("MACOBLOX_AUDIO=0".into());
    }

    // Pass deep link launch URL or place ID if provided
    if let Ok(url) = std::env::var("MACOBLOX_LAUNCH_URL") {
        if !url.trim().is_empty() {
            args.push(url.trim().to_string());
        }
    }
    if let Ok(place_id) = std::env::var("MACOBLOX_PLACE_ID") {
        let trimmed = place_id.trim();
        if !trimmed.is_empty() {
            if std::env::var("MACOBLOX_LAUNCH_URL").is_err() {
                args.push(format!("roblox://placeId={}", trimmed));
            }
            args.push(format!("--id={}", trimmed));
        }
    }

    // Check wrapper tools: GameMode, Gamescope, MangoHud
    let disable_gamemode = std::env::var("CRABBLOX_DISABLE_GAMEMODE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let gamemode_bin = if !disable_gamemode {
        find_binary("gamemoderun")
    } else {
        None
    };

    let mangohud_requested = std::env::var("CRABBLOX_MANGOHUD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mangohud_bin = if mangohud_requested {
        let bin = find_binary("mangohud");
        if bin.is_none() {
            eprintln!("Warning: CRABBLOX_MANGOHUD=1 is set, but 'mangohud' binary was not found.");
        }
        bin
    } else {
        None
    };

    let gamescope_requested = std::env::var("CRABBLOX_GAMESCOPE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let gamescope_bin = if gamescope_requested {
        let bin = find_binary("gamescope");
        if bin.is_none() {
            eprintln!("Warning: CRABBLOX_GAMESCOPE=1 is set, but 'gamescope' binary was not found.");
        }
        bin
    } else {
        None
    };

    if gamemode_bin.is_some() {
        println!("GameMode enabled: launching with gamemoderun");
    }
    if mangohud_bin.is_some() {
        println!("MangoHud enabled: launching with mangohud");
    }
    if gamescope_bin.is_some() {
        println!("Gamescope enabled: launching with gamescope");
    }

    // Build the execution command chain:
    // Base command: darling <args...>
    let darling_cmd = find_binary("darling")
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "darling".to_string());
    let mut exec_chain = vec![darling_cmd];
    exec_chain.extend(args);

    // If MangoHud is enabled, wrap darling
    if let Some(ref bin) = mangohud_bin {
        let mut wrapped = vec![bin.to_string_lossy().to_string()];
        wrapped.extend(exec_chain);
        exec_chain = wrapped;
    }

    // If Gamescope is enabled, wrap command with gamescope [--]
    if let Some(ref bin) = gamescope_bin {
        let mut wrapped = vec![bin.to_string_lossy().to_string()];
        if let Ok(gs_args) = std::env::var("CRABBLOX_GAMESCOPE_ARGS") {
            wrapped.extend(gs_args.split_whitespace().map(|s| s.to_string()));
        }
        wrapped.push("--".to_string());
        wrapped.extend(exec_chain);
        exec_chain = wrapped;
    }

    // If GameMode is enabled, wrap outer command with gamemoderun
    if let Some(ref bin) = gamemode_bin {
        let mut wrapped = vec![bin.to_string_lossy().to_string()];
        wrapped.extend(exec_chain);
        exec_chain = wrapped;
    }

    let program = exec_chain.remove(0);
    let mut cmd = Command::new(program);
    cmd.args(&exec_chain)
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file));

    cmd.env("DPREFIX", &paths.darling_prefix);
    cmd.env("EGL_PLATFORM", "x11");
    if let Ok(user) = std::env::var("USER") {
        cmd.env("USER", user);
    }
    if let Ok(display) = std::env::var("DISPLAY") {
        cmd.env("DISPLAY", display);
    } else {
        cmd.env("DISPLAY", ":0");
    }
    if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
        cmd.env("WAYLAND_DISPLAY", wayland);
    }
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        cmd.env("XDG_RUNTIME_DIR", runtime_dir);
    }
    if let Ok(pulse) = std::env::var("PULSE_SERVER") {
        cmd.env("PULSE_SERVER", pulse);
    }
    if let Ok(dbus) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        cmd.env("DBUS_SESSION_BUS_ADDRESS", dbus);
    }
    if let Ok(noroot) = std::env::var("MACOBLOX_NOROOT_LIB") {
        cmd.env("LD_PRELOAD", noroot);
    }

    // PipeWire & PulseAudio latency environment
    cmd.env("PIPEWIRE_LATENCY", "256/44100");
    cmd.env("PULSE_LATENCY_MSEC", "50");

    // GPU optimizations and PRIME offload variables
    for (k, v) in gpu_env.env_vars {
        cmd.env(k, v);
    }

    let child = cmd.spawn()?;

    let rpc = Some(crate::discord_rpc::DiscordRpc::start(
        "Playing Roblox via Crabblox".to_string(),
        "by Monster Dev".to_string(),
    ));

    Ok(RobloxSession {
        dns,
        audio,
        rpc,
        log_path,
        child,
        diagnostics_performed: false,
    })
}

#[allow(dead_code)]
pub fn find_binary(name: &str) -> Option<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for dir in &["/usr/bin", "/usr/sbin", "/usr/local/bin", "/bin", "/sbin"] {
        let candidate = Path::new(dir).join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if Command::new(name)
        .arg("/bin/true")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Some(PathBuf::from(name));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashCategory {
    OutOfMemory,
    SegmentationFault,
    DyldLinkFailure,
    PermissionOrSandbox,
    DarlingServerFailure,
    DisplayOrVulkan,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashDiagnosis {
    pub category: CrashCategory,
    pub title: String,
    pub description: String,
    pub advice: Vec<String>,
}

#[allow(dead_code)]
pub fn scan_crash_diagnostics(log_path: &Path) -> Vec<CrashDiagnosis> {
    scan_crash_diagnostics_with_status(log_path, None)
}

pub fn scan_crash_diagnostics_with_status(
    log_path: &Path,
    status: Option<&std::process::ExitStatus>,
) -> Vec<CrashDiagnosis> {
    let exit_code = status.and_then(|s| s.code());
    let signal = {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            status.and_then(|s| s.signal())
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    scan_crash_diagnostics_with_exit(log_path, exit_code, signal)
}

pub fn scan_crash_diagnostics_with_exit(
    log_path: &Path,
    exit_code: Option<i32>,
    signal: Option<i32>,
) -> Vec<CrashDiagnosis> {
    eprintln!("\n=======================================================");
    eprintln!("        Crabblox Crash Diagnostics & Troubleshooting   ");
    eprintln!("=======================================================");
    if let Some(code) = exit_code {
        eprintln!("Process exited with status code: {}", code);
    }
    if let Some(sig) = signal {
        eprintln!("Process terminated by signal: {}", sig);
    }
    eprintln!("Scanning log file for crash signatures: {:?}", log_path);

    let content = match read_log_tail(log_path, 2 * 1024 * 1024) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Could not read log file: {e}");
            eprintln!("=======================================================\n");
            return Vec::new();
        }
    };

    let content_lower = content.to_lowercase();
    let mut diagnoses = Vec::new();

    // 1. Out-of-Memory (OOM)
    let is_oom = signal == Some(9)
        || exit_code == Some(137)
        || content_lower.contains("out of memory")
        || content_lower.contains("cannot allocate memory")
        || content_lower.contains("oom-killer")
        || content_lower.contains("killed process")
        || content_lower.contains("std::bad_alloc")
        || content_lower.contains("allocation failed")
        || content_lower.contains("virtualalloc failed")
        || content_lower.contains("mmap failed: cannot allocate memory")
        || content_lower.contains("darling: memory exhausted")
        || content_lower.contains("kerroutofmemory");

    if is_oom {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::OutOfMemory,
            title: "Out-of-Memory (OOM) Termination".to_string(),
            description: "Roblox or Darling ran out of RAM or swap space and was terminated by Linux (OOM-killer) or memory allocator.".to_string(),
            advice: vec![
                "Close memory-heavy applications (web browsers, Discord, IDEs) before playing.".to_string(),
                "Increase Linux swap space (e.g. enable zram: 'sudo zramctl' or add a swap file: 'sudo fallocate -l 4G /swapfile && sudo mkswap /swapfile && sudo swapon /swapfile').".to_string(),
                "Lower graphics quality in Fast Flags (ClientAppSettings.json: DFIntDebugFRMQualityLevelOverride = 1).".to_string(),
            ],
        });
    }

    // 2. Dynamic linker failure (dyld)
    let is_dyld = content_lower.contains("dyld: symbol not found")
        || content_lower.contains("symbol not found:")
        || content_lower.contains("symbol lookup error")
        || content_lower.contains("lazy symbol binding failed")
        || content_lower.contains("dyld: library not loaded")
        || content_lower.contains("reason: image not found")
        || content_lower.contains("incompatible library version")
        || content_lower.contains("mach-o file, but wrong architecture")
        || content_lower.contains("file not found: libmacobloxshims.dylib");

    if is_dyld {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::DyldLinkFailure,
            title: "Dynamic Linker Failure (dyld symbol / library lookup error)".to_string(),
            description: "A required macOS Darwin dynamic symbol or stub framework was not found at runtime.".to_string(),
            advice: vec![
                "Rebuild the MacOBlox shims: run './build_debug_shim.sh' in the project directory.".to_string(),
                "Verify stub frameworks (CoreML, CoreHaptics, DeviceCheck) are present in Darling prefix System/Library/Frameworks.".to_string(),
                "Ensure DYLD_INSERT_LIBRARIES points to a valid libMacOBloxShims.dylib.".to_string(),
            ],
        });
    }

    // 3. Segmentation fault / Memory access violation
    let is_segfault = signal == Some(11)
        || exit_code == Some(139)
        || signal == Some(10)
        || exit_code == Some(138)
        || (!is_oom && (signal == Some(6) || exit_code == Some(134)))
        || content_lower.contains("segmentation fault")
        || content_lower.contains("sigsegv")
        || content_lower.contains("exc_bad_access")
        || content_lower.contains("segfault at")
        || content_lower.contains("code=segv")
        || content_lower.contains("bus error")
        || content_lower.contains("sigbus")
        || content_lower.contains("abort trap: 6")
        || content_lower.contains("assertion failed");

    if is_segfault {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::SegmentationFault,
            title: "Memory Access Violation (Segmentation Fault / SIGSEGV)".to_string(),
            description: "Roblox or Darling crashed while accessing an invalid memory address or encountered a critical assertion failure.".to_string(),
            advice: vec![
                "Clear the Mesa shader cache: rm -rf ~/.cache/mesa_shader_cache".to_string(),
                "Clear Crabblox Vulkan pipeline cache: rm -rf ~/.cache/crabblox/vulkan_cache".to_string(),
                "Restart the Darling container daemon: crabblox run --restart-darling".to_string(),
                "Update your host GPU graphics drivers (Mesa / NVIDIA).".to_string(),
                "If running under Wayland, test under X11 or with Gamescope (CRABBLOX_GAMESCOPE=1).".to_string(),
            ],
        });
    }

    // 4. Permission / Sandboxing issues
    let is_perm = exit_code == Some(126)
        || content_lower.contains("permission denied")
        || content_lower.contains("operation not permitted")
        || content_lower.contains("eacces")
        || content_lower.contains("eperm")
        || content_lower.contains("access denied")
        || content_lower.contains("failed to create socket: permission denied")
        || content_lower.contains("cannot bind to port: permission denied")
        || content_lower.contains("requires root or darling-mach kernel module")
        || content_lower.contains("noroot library failed")
        || content_lower.contains("failed to open /dev/");

    if is_perm {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::PermissionOrSandbox,
            title: "Permission Denied / Sandboxing Failure".to_string(),
            description: "The process could not access required files, devices, or sockets due to permission restrictions.".to_string(),
            advice: vec![
                "Fix Darling prefix permissions: chmod -R u+rwX ~/.darling".to_string(),
                "Add your user to required hardware groups: sudo usermod -aG audio,video,render $USER".to_string(),
                "Enable unprivileged user namespaces: sudo sysctl -w kernel.unprivileged_userns_clone=1".to_string(),
                "If running inside a container or Flatpak, check sandbox device and filesystem permissions.".to_string(),
            ],
        });
    }

    // 5. Darlingserver / shellspawn communication failure
    let is_ds = content_lower.contains("cannot connect to darlingserver")
        || content_lower.contains("failed to connect to darlingserver")
        || content_lower.contains("darlingserver communication failure")
        || content_lower.contains("communication error with darlingserver")
        || content_lower.contains("darlingserver is not responding")
        || content_lower.contains("shellspawn")
        || content_lower.contains("error connecting to shellspawn")
        || (content_lower.contains("darlingserver")
            && (content_lower.contains("connection refused")
                || content_lower.contains("broken pipe")
                || content_lower.contains("dead")
                || content_lower.contains("died")
                || content_lower.contains("socket error")));

    if is_ds {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::DarlingServerFailure,
            title: "Darlingserver / Shellspawn Container Failure".to_string(),
            description: "The Darling container daemon (darlingserver/shellspawn) is not running, crashed, or could not create its socket in ~/.darling/var/run/.".to_string(),
            advice: vec![
                "Check ~/.darling directory permissions (if previously run with sudo): sudo chown -R $USER:$USER ~/.darling".to_string(),
                "Restart the Darling container daemon: darling shutdown && crabblox run --restart-darling".to_string(),
                "Kill any lingering processes: killall -9 darlingserver launchd 2>/dev/null".to_string(),
                "If on Ubuntu 24.04/Debian with AppArmor unprivileged user namespace restrictions: sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0".to_string(),
                "If prefix is corrupted, perform a fresh reset: darling shutdown && rm -rf ~/.darling && darling shell /bin/echo ok".to_string(),
            ],
        });
    }

    // 6. Display / Vulkan failure
    let is_display = content_lower.contains("cannot open display")
        || content_lower.contains("failed to open display")
        || content_lower.contains("x11 connection rejected")
        || content_lower.contains("no protocol specified")
        || content_lower.contains("vk_error_")
        || content_lower.contains("vkcreateinstance")
        || content_lower.contains("unable to find a compatible vulkan")
        || content_lower.contains("libgl error")
        || content_lower.contains("egl_bad_alloc");

    if is_display {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::DisplayOrVulkan,
            title: "Display Server or Vulkan Graphics Failure".to_string(),
            description: "Failed to connect to the X11/Wayland display server or initialize the 3D graphics pipeline.".to_string(),
            advice: vec![
                "Ensure DISPLAY (or WAYLAND_DISPLAY) is properly set.".to_string(),
                "Authorize local X11 access: xhost +local:".to_string(),
                "Run 'crabblox doctor' to check Vulkan drivers and 3D acceleration.".to_string(),
                "Ensure both 32-bit and 64-bit Vulkan drivers are installed.".to_string(),
            ],
        });
    }

    if diagnoses.is_empty() && (exit_code.map(|c| c != 0).unwrap_or(false) || signal.is_some()) {
        diagnoses.push(CrashDiagnosis {
            category: CrashCategory::Unknown,
            title: "Unclassified Process Failure".to_string(),
            description: format!(
                "Roblox exited abnormally (exit code: {:?}, signal: {:?}) without matching a known crash signature.",
                exit_code, signal
            ),
            advice: vec![
                "Run 'crabblox doctor' to verify system readiness (Darling, Vulkan, audio, limits).".to_string(),
                "Review the log snippet below for error details or consult the Crabblox community.".to_string(),
            ],
        });
    }

    if !diagnoses.is_empty() {
        eprintln!("Identified Crash Signatures & Recommendations:\n");
        for diag in &diagnoses {
            eprintln!("• {}:\n  {}\n  Troubleshooting recommendations:", diag.title, diag.description);
            for adv in &diag.advice {
                eprintln!("    - {}", adv);
            }
            eprintln!();
        }
    } else {
        eprintln!("No specific known signature matched. Review the log snippet below.\n");
    }

    let lines: Vec<&str> = content.lines().collect();
    let tail_count = 15.min(lines.len());
    if tail_count > 0 {
        eprintln!("--- Log Tail (last {} lines) ---", tail_count);
        for line in &lines[lines.len() - tail_count..] {
            eprintln!("  {}", line);
        }
        eprintln!("--- End of Log Tail ---");
    }
    eprintln!("=======================================================\n");

    diagnoses
}

fn read_log_tail(log_path: &Path, max_bytes: u64) -> std::io::Result<String> {
    let mut file = fs::File::open(log_path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    if len > max_bytes {
        file.seek(SeekFrom::Start(len - max_bytes))?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(String::from_utf8_lossy(&buf).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_find_binary() {
        assert!(find_binary("sh").is_some());
        assert!(find_binary("nonexistent_binary_xyz_123").is_none());
    }

    #[test]
    fn test_scan_crash_diagnostics_segfault() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "Starting Darling...").unwrap();
        writeln!(file, "RobloxPlayer[1234]: Segmentation fault: 11 (SIGSEGV)").unwrap();
        writeln!(file, "darlingserver: child exited with signal 11").unwrap();

        let diags = scan_crash_diagnostics(temp.path());
        assert!(diags.iter().any(|d| d.category == CrashCategory::SegmentationFault));
    }

    #[test]
    fn test_scan_crash_diagnostics_dyld() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "dyld: Symbol not found: _OBJC_CLASS_$_CoreML").unwrap();
        writeln!(file, "dyld: Library not loaded: @rpath/CoreML.framework/CoreML").unwrap();

        let diags = scan_crash_diagnostics(temp.path());
        assert!(diags.iter().any(|d| d.category == CrashCategory::DyldLinkFailure));
    }

    #[test]
    fn test_scan_crash_diagnostics_darlingserver() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "Cannot connect to darlingserver: Connection refused").unwrap();
        writeln!(file, "Failed to connect to darlingserver at .darlingserver.sock").unwrap();

        let diags = scan_crash_diagnostics(temp.path());
        assert!(diags.iter().any(|d| d.category == CrashCategory::DarlingServerFailure));
    }

    #[test]
    fn test_scan_crash_diagnostics_shellspawn() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "Error connecting to shellspawn in the container (/home/mein/.darling/var/run/shellspawn.sock): No such file or directory").unwrap();

        let diags = scan_crash_diagnostics(temp.path());
        assert!(diags.iter().any(|d| d.category == CrashCategory::DarlingServerFailure));
    }

    #[test]
    fn test_scan_crash_diagnostics_oom_log() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "RobloxPlayer[2048]: fatal error: out of memory (cannot allocate 268435456 bytes)").unwrap();
        writeln!(file, "darlingserver: memory exhausted").unwrap();

        let diags = scan_crash_diagnostics(temp.path());
        assert!(diags.iter().any(|d| d.category == CrashCategory::OutOfMemory));
    }

    #[test]
    fn test_scan_crash_diagnostics_oom_signal() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let diags = scan_crash_diagnostics_with_exit(temp.path(), Some(137), Some(9));
        assert!(diags.iter().any(|d| d.category == CrashCategory::OutOfMemory));
    }

    #[test]
    fn test_scan_crash_diagnostics_permission() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp.as_file();
        writeln!(file, "darling: failed to open /dev/mach: Permission denied").unwrap();

        let diags = scan_crash_diagnostics_with_exit(temp.path(), Some(126), None);
        assert!(diags.iter().any(|d| d.category == CrashCategory::PermissionOrSandbox));
    }

    #[test]
    fn test_scan_crash_diagnostics_status_segfault() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let diags = scan_crash_diagnostics_with_exit(temp.path(), Some(139), Some(11));
        assert!(diags.iter().any(|d| d.category == CrashCategory::SegmentationFault));
    }

    #[test]
    fn test_raise_fd_limit() {
        raise_fd_limit();
    }

    #[test]
    fn test_check_disk_space() {
        let temp = tempfile::tempdir().unwrap();
        // Should succeed for 1 MB
        assert!(check_disk_space(temp.path(), 1024 * 1024).is_ok());
        // Should fail for an impossible 1000 Terabytes
        assert!(check_disk_space(temp.path(), 1000 * 1024 * 1024 * 1024 * 1024).is_err());
    }

    #[test]
    fn test_detect_gpu_environment() {
        let env = detect_gpu_environment();
        assert!(env.env_vars.iter().any(|(k, _)| k == "VK_PIPELINE_CACHE_ENABLE"));
    }

    #[test]
    fn test_is_pipewire_active() {
        let _ = HostAudio::is_pipewire_active();
    }
}
