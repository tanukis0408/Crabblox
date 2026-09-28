use crate::dns::DnsForwarder;
use crate::paths::Paths;
use chrono::Local;
use std::ffi::CString;
use std::fs;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

const LAUNCH_SCRIPT: &str = r#"
app_dir=$1 shim_dir=$2; shift 2
for kv in "$@"; do export "$kv"; done
cd "$app_dir" || exit 1
export DYLD_FORCE_FLAT_NAMESPACE=1
export DYLD_INSERT_LIBRARIES="$shim_dir/libMacOBloxShims.dylib"
export DYLD_LIBRARY_PATH="$shim_dir:$app_dir"
exec ./RobloxPlayer
"#;

pub struct HostAudio {
    pub fifo_path: PathBuf,
    pub keep_file: fs::File,
    pub player: Child,
}

impl HostAudio {
    pub fn start(cache_dir: &std::path::Path) -> Option<Self> {
        let player_bin = if Command::new("pw-cat").arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
            || std::path::Path::new("/usr/bin/pw-cat").exists()
        {
            "pw-cat"
        } else if Command::new("pacat").arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
            || std::path::Path::new("/usr/bin/pacat").exists()
        {
            "pacat"
        } else {
            return None;
        };

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

        // Open read-write so the FIFO stays open continuously
        let keep_file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&fifo_path)
            .ok()?;

        let mut cmd = Command::new(player_bin);
        if player_bin == "pw-cat" {
            cmd.args([
                "--playback", "--raw", "--format", "f32", "--rate", "44100",
                "--channels", "2", "--latency", "40ms", "--media-role", "Game",
                "-P", "{ application.name = \"Roblox\" media.name = \"Roblox (Mac O’ Blox)\" }",
                fifo_path.to_str()?,
            ]);
        } else {
            cmd.args([
                "--playback", "--raw", "--format=float32le", "--rate=44100",
                "--channels=2", "--latency-msec=40", "--client-name=Roblox",
                "--stream-name=Roblox (Mac O’ Blox)", "--property=media.role=game",
                fifo_path.to_str()?,
            ]);
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
        let _ = fs::remove_file(&self.fifo_path);
    }
}

pub struct RobloxSession {
    pub dns: DnsForwarder,
    pub audio: Option<HostAudio>,
    pub rpc: Option<crate::discord_rpc::DiscordRpc>,
    pub log_path: PathBuf,
    pub child: Child,
}

impl Drop for RobloxSession {
    fn drop(&mut self) {
        if let Some(ref r) = self.rpc {
            r.stop();
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

pub fn get_parent_pid(pid: i32) -> Option<i32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit(')').next()?;
    let mut parts = after_comm.split_whitespace();
    let _state = parts.next()?;
    let ppid_str = parts.next()?;
    ppid_str.parse::<i32>().ok()
}

pub fn clear_stale_darling(prefix: &std::path::Path) {
    let init_pid_path = prefix.join(".init.pid");
    let sock_path = prefix.join(".darlingserver.sock");

    if let Ok(content) = fs::read_to_string(&init_pid_path) {
        if let Ok(pid) = content.trim().parse::<i32>() {
            let state = process_state(pid);
            if state.is_none() || state == Some('Z') {
                if state == Some('Z') {
                    if let Some(ppid) = get_parent_pid(pid) {
                        unsafe {
                            libc::kill(ppid, libc::SIGKILL);
                        }
                        let _ = Command::new("sudo")
                            .args(["-n", "kill", "-9", &ppid.to_string()])
                            .status();
                    }
                }
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
}

pub fn roblox_pids() -> Vec<i32> {
    let mut pids = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<i32>() {
                let cmdline = entry.path().join("cmdline");
                if let Ok(bytes) = fs::read(cmdline) {
                    let cmd = String::from_utf8_lossy(&bytes);
                    if (cmd.contains("RobloxPlayer") || cmd.contains("RobloxCrashHandler"))
                        && !cmd.contains("darling shell")
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
    let pids = roblox_pids();
    for pid in &pids {
        unsafe {
            libc::kill(*pid, libc::SIGTERM);
        }
    }
    if !pids.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(500));
        for pid in roblox_pids() {
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

    let shim_parent = paths.shim_dylib().parent().unwrap().to_path_buf();
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
        "mesa_glthread=true".into(),
        "MACOBLOX_MOUSE_SENSITIVITY=1.00".into(),
    ];

    let icon_file = paths.cache_dir.join("icon.argb");
    if icon_file.exists() {
        args.push(format!("MACOBLOX_ICON_ARGB=/Volumes/SystemRoot{}", icon_file.display()));
    }

    if let Some(ref a) = audio {
        args.push(format!("MACOBLOX_AUDIO_FIFO=/Volumes/SystemRoot{}", a.fifo_path.display()));
    } else {
        args.push("MACOBLOX_AUDIO=0".into());
    }

    let mut cmd = Command::new("darling");
    cmd.args(&args)
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
    })
}
