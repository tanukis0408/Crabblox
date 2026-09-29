use crate::paths::Paths;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn is_pass(&self) -> bool {
        matches!(self, CheckStatus::Pass)
    }
    pub fn is_warn(&self) -> bool {
        matches!(self, CheckStatus::Warn)
    }
    pub fn is_fail(&self) -> bool {
        matches!(self, CheckStatus::Fail)
    }

    pub fn tag(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "[✓ PASS]",
            CheckStatus::Warn => "[! WARN]",
            CheckStatus::Fail => "[✗ FAIL]",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub category: &'static str,
    pub name: &'static str,
    pub status: CheckStatus,
    pub summary: String,
    pub details: Vec<String>,
    pub remediation: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub checks: Vec<CheckItem>,
}

impl DoctorReport {
    pub fn run(paths: &Paths) -> Self {
        let checks = vec![
            check_darling(paths),
            check_vulkan(),
            check_sound(),
            check_system_limits(),
            check_disk_space(paths),
            check_network(),
        ];
        DoctorReport { checks }
    }

    pub fn summary_counts(&self) -> (usize, usize, usize) {
        let mut p = 0;
        let mut w = 0;
        let mut f = 0;
        for c in &self.checks {
            match c.status {
                CheckStatus::Pass => p += 1,
                CheckStatus::Warn => w += 1,
                CheckStatus::Fail => f += 1,
            }
        }
        (p, w, f)
    }

    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|c| c.status.is_fail())
    }

    pub fn has_warnings(&self) -> bool {
        self.checks.iter().any(|c| c.status.is_warn())
    }

    pub fn print_cli(&self) {
        println!("\n=======================================================");
        println!("             Crabblox System Doctor (by Monster Dev)   ");
        println!("=======================================================");
        println!("Checking system components and environment readiness...\n");

        for item in &self.checks {
            println!("{} {} ({})", item.status.tag(), item.name, item.category);
            println!("         Summary: {}", item.summary);
            for d in &item.details {
                println!("         • {}", d);
            }
            if let Some(ref rem) = item.remediation {
                println!("         >>> Remediation Advice:");
                for line in rem.lines() {
                    println!("             {}", line);
                }
            }
            println!();
        }

        let (p, w, f) = self.summary_counts();
        println!("-------------------------------------------------------");
        println!("Doctor Summary: {} passed, {} warning(s), {} failed.", p, w, f);
        if f == 0 && w == 0 {
            println!("All checks passed! System is 100% ready to launch Roblox.");
        } else if f == 0 {
            println!("All critical checks passed with non-fatal warnings.");
        } else {
            println!("One or more critical checks failed. Please address the remediation steps above.");
        }
        println!("=======================================================\n");
    }

    pub fn to_dialog_text(&self) -> String {
        let (p, w, f) = self.summary_counts();
        let mut out = format!(
            "Результаты проверки системы Crabblox:\n{} успешно, {} предупреждений, {} ошибок\n\n",
            p, w, f
        );
        for item in &self.checks {
            out.push_str(&format!("{} {}\n  {}\n", item.status.tag(), item.name, item.summary));
            for d in &item.details {
                out.push_str(&format!("  • {}\n", d));
            }
            if let Some(ref rem) = item.remediation {
                out.push_str("  Рекомендация:\n");
                for line in rem.lines() {
                    out.push_str(&format!("    {}\n", line));
                }
            }
            out.push('\n');
        }
        out
    }
}

pub fn check_darling(paths: &Paths) -> CheckItem {
    let binary = crate::runner::find_binary("darling");
    let mut details = Vec::new();
    let mut remediation = None;

    let (status, summary) = match binary {
        None => {
            details.push("'darling' binary not found in PATH or standard system directories.".to_string());
            remediation = Some(
                "Install Darling for your distribution (or build from source: https://github.com/darlinghq/darling).\n\
                 Ensure the darling executable is located in your PATH (e.g. /usr/bin or /usr/local/bin)."
                    .to_string(),
            );
            (
                CheckStatus::Fail,
                "Darling binary is not installed or not found in PATH".to_string(),
            )
        }
        Some(bin_path) => {
            details.push(format!("Binary found: {}", bin_path.display()));
            let ver_out = Command::new(&bin_path).arg("--version").output();
            let ver_str = match ver_out {
                Ok(out) if out.status.success() => {
                    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if s.is_empty() {
                        String::from_utf8_lossy(&out.stderr).trim().to_string()
                    } else {
                        s
                    }
                }
                _ => "Unknown version".to_string(),
            };
            if !ver_str.is_empty() {
                details.push(format!(
                    "Version: {}",
                    ver_str.lines().next().unwrap_or(&ver_str)
                ));
            }
            details.push(format!("Prefix path: {}", paths.darling_prefix.display()));

            let is_running = crate::runner::is_darlingserver_running(&paths.darling_prefix);
            let has_init_pid = paths.darling_prefix.join(".init.pid").exists();
            let has_sock = paths.darling_prefix.join(".darlingserver.sock").exists();

            if is_running {
                details.push("darlingserver: active and running".to_string());
                (
                    CheckStatus::Pass,
                    format!(
                        "Darling is operational ({})",
                        ver_str.lines().next().unwrap_or("ready")
                    ),
                )
            } else if has_init_pid || has_sock {
                details.push(
                    "darlingserver: stopped, but stale socket or pid file detected in prefix"
                        .to_string(),
                );
                remediation = Some(
                    "Run 'crabblox run --restart-darling' or use 'Перезапустить Darling' in settings to clean up stale state."
                        .to_string(),
                );
                (
                    CheckStatus::Warn,
                    "Darling installed, but stale container socket detected".to_string(),
                )
            } else {
                details.push(
                    "darlingserver: idle (will start automatically on game launch)".to_string(),
                );
                (
                    CheckStatus::Pass,
                    format!(
                        "Darling is installed and ready ({})",
                        ver_str.lines().next().unwrap_or("ready")
                    ),
                )
            }
        }
    };

    CheckItem {
        category: "Darling Container",
        name: "Darling Runtime",
        status,
        summary,
        details,
        remediation,
    }
}

pub fn check_vulkan() -> CheckItem {
    let mut details = Vec::new();

    let search_dirs = [
        Path::new("/usr/share/vulkan/icd.d"),
        Path::new("/etc/vulkan/icd.d"),
        Path::new("/usr/local/share/vulkan/icd.d"),
        Path::new("/etc/vulkan/implicit_layer.d"),
    ];

    let mut icd_files = Vec::new();
    for dir in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if !icd_files.contains(&fname) {
                        icd_files.push(fname);
                    }
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        let user_icd = home.join(".local/share/vulkan/icd.d");
        if let Ok(entries) = std::fs::read_dir(&user_icd) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if !icd_files.contains(&fname) {
                        icd_files.push(fname);
                    }
                }
            }
        }
    }

    let loader_candidates = [
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/usr/lib/libvulkan.so.1",
        "/usr/local/lib/libvulkan.so.1",
    ];
    let loader_found = loader_candidates.iter().find(|p| Path::new(p).exists());
    let vulkaninfo_found = crate::runner::find_binary("vulkaninfo");

    let has_loader = loader_found.is_some() || vulkaninfo_found.is_some();
    if let Some(p) = loader_found {
        details.push(format!("Vulkan loader: {}", p));
    } else if let Some(ref vi) = vulkaninfo_found {
        details.push(format!("Vulkan tools: {}", vi.display()));
    } else {
        details.push("Vulkan loader (libvulkan.so.1): not found in standard paths".to_string());
    }

    if icd_files.is_empty() {
        details.push(
            "No Vulkan ICD manifests (.json) found in /usr/share/vulkan/icd.d or /etc/vulkan/icd.d."
                .to_string(),
        );
        let remediation = Some(
            "Install Vulkan drivers for your GPU:\n\
             • Intel/AMD: sudo apt install mesa-vulkan-drivers libvulkan1\n\
             • NVIDIA: install the proprietary nvidia driver (nvidia-driver) with Vulkan support\n\
             • Arch: sudo pacman -S vulkan-radeon / vulkan-intel / nvidia-utils"
                .to_string(),
        );
        return CheckItem {
            category: "Graphics & 3D",
            name: "Vulkan Drivers",
            status: CheckStatus::Fail,
            summary: "No Vulkan ICD manifests found (hardware acceleration unavailable)"
                .to_string(),
            details,
            remediation,
        };
    }

    details.push(format!(
        "ICD manifests detected ({}): {}",
        icd_files.len(),
        icd_files.join(", ")
    ));

    let only_software = icd_files
        .iter()
        .all(|f| f.to_lowercase().contains("lvp"));

    if only_software {
        let remediation = Some(
            "Only LLVMpipe (CPU software rendering) was detected. Install hardware-accelerated Vulkan drivers for your GPU (Mesa radv/iris or NVIDIA)."
                .to_string(),
        );
        CheckItem {
            category: "Graphics & 3D",
            name: "Vulkan Drivers",
            status: CheckStatus::Warn,
            summary: "Only CPU software Vulkan driver (LLVMpipe) detected; performance will be low"
                .to_string(),
            details,
            remediation,
        }
    } else if !has_loader {
        let remediation = Some(
            "Install the Vulkan loader library: sudo apt install libvulkan1 (or equivalent for your distribution)."
                .to_string(),
        );
        CheckItem {
            category: "Graphics & 3D",
            name: "Vulkan Drivers",
            status: CheckStatus::Warn,
            summary: "Vulkan ICD manifests present, but libvulkan.so.1 loader was not found"
                .to_string(),
            details,
            remediation,
        }
    } else {
        CheckItem {
            category: "Graphics & 3D",
            name: "Vulkan Drivers",
            status: CheckStatus::Pass,
            summary: format!(
                "Vulkan hardware acceleration active ({} ICDs detected)",
                icd_files.len()
            ),
            details,
            remediation: None,
        }
    }
}

pub fn check_sound() -> CheckItem {
    let mut details = Vec::new();

    let is_pw = crate::runner::HostAudio::is_pipewire_active();
    let pw_cat = crate::runner::find_binary("pw-cat");
    let pw_play = crate::runner::find_binary("pw-play");
    let pacat = crate::runner::find_binary("pacat");
    let paplay = crate::runner::find_binary("paplay");

    let chosen_bin = if is_pw {
        pw_cat
            .as_ref()
            .or(pw_play.as_ref())
            .or(pacat.as_ref())
            .or(paplay.as_ref())
    } else {
        pacat
            .as_ref()
            .or(paplay.as_ref())
            .or(pw_cat.as_ref())
            .or(pw_play.as_ref())
    };

    details.push(format!(
        "Active sound server: {}",
        if is_pw {
            "PipeWire (active low-latency server)"
        } else {
            "PulseAudio / ALSA"
        }
    ));

    if let Some(bin) = chosen_bin {
        details.push(format!("Playback binary: {}", bin.display()));
    } else {
        details.push(
            "No sound utility found (pw-cat, pw-play, pacat, paplay not found in PATH)."
                .to_string(),
        );
    }

    let dev_snd = Path::new("/dev/snd");
    let dev_snd_exists = dev_snd.exists();
    let dev_snd_accessible = if dev_snd_exists {
        std::fs::read_dir(dev_snd)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false)
    } else {
        false
    };

    if dev_snd_exists {
        details.push(format!(
            "/dev/snd access: {}",
            if dev_snd_accessible {
                "OK (accessible)"
            } else {
                "Restricted"
            }
        ));
    } else {
        details.push("/dev/snd: not found (running in restricted environment?)".to_string());
    }

    if chosen_bin.is_none() {
        let remediation = Some(
            "Install PipeWire or PulseAudio CLI tools:\n\
             • Ubuntu/Debian: sudo apt install pipewire-bin pipewire-pulse pulseaudio-utils\n\
             • Fedora: sudo dnf install pipewire-utils pulseaudio-utils\n\
             • Arch: sudo pacman -S pipewire-utils libpulse"
                .to_string(),
        );
        CheckItem {
            category: "Sound Subsystem",
            name: "Audio Server & Playback",
            status: CheckStatus::Warn,
            summary: "No audio playback binary found (in-game audio disabled)".to_string(),
            details,
            remediation,
        }
    } else if !dev_snd_accessible && dev_snd_exists {
        let remediation = Some(
            "Ensure your user has access to sound devices: sudo usermod -aG audio $USER".to_string(),
        );
        CheckItem {
            category: "Sound Subsystem",
            name: "Audio Server & Playback",
            status: CheckStatus::Warn,
            summary: "Audio binary found, but /dev/snd device permissions are restricted".to_string(),
            details,
            remediation,
        }
    } else {
        details.push(
            "Low-latency audio pipeline: 50ms latency target, 256KB FIFO buffer headroom"
                .to_string(),
        );
        CheckItem {
            category: "Sound Subsystem",
            name: "Audio Server & Playback",
            status: CheckStatus::Pass,
            summary: format!(
                "{} audio server operational ({})",
                if is_pw { "PipeWire" } else { "PulseAudio" },
                chosen_bin
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
            ),
            details,
            remediation: None,
        }
    }
}

pub fn check_system_limits() -> CheckItem {
    let mut details = Vec::new();

    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let res = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) };

    if res != 0 {
        details.push("Failed to query RLIMIT_NOFILE via libc.".to_string());
        return CheckItem {
            category: "System Limits",
            name: "File Descriptors (RLIMIT_NOFILE)",
            status: CheckStatus::Warn,
            summary: "Could not determine system file descriptor limits".to_string(),
            details,
            remediation: None,
        };
    }

    details.push(format!("Soft limit: {} open files", rlim.rlim_cur));
    details.push(format!("Hard limit: {} open files", rlim.rlim_max));
    details.push("Crabblox auto-boost: raises soft limit up to 65536 on launch.".to_string());

    if rlim.rlim_max < 4096 {
        let remediation = Some(
            "Increase file descriptor limits in /etc/security/limits.conf:\n\
             * soft nofile 65536\n\
             * hard nofile 65536\n\
             Or in /etc/systemd/user.conf: DefaultLimitNOFILE=65536:524288"
                .to_string(),
        );
        CheckItem {
            category: "System Limits",
            name: "File Descriptors (RLIMIT_NOFILE)",
            status: CheckStatus::Fail,
            summary: format!(
                "Hard file descriptor limit is dangerously low ({})",
                rlim.rlim_max
            ),
            details,
            remediation,
        }
    } else if rlim.rlim_max < 65536 {
        let remediation = Some(
            "Increase limits in /etc/security/limits.conf (* soft nofile 65536, * hard nofile 65536) for best stability with large Roblox games."
                .to_string(),
        );
        CheckItem {
            category: "System Limits",
            name: "File Descriptors (RLIMIT_NOFILE)",
            status: CheckStatus::Warn,
            summary: format!(
                "File descriptor limit is moderate (hard: {}, soft: {})",
                rlim.rlim_max, rlim.rlim_cur
            ),
            details,
            remediation,
        }
    } else {
        CheckItem {
            category: "System Limits",
            name: "File Descriptors (RLIMIT_NOFILE)",
            status: CheckStatus::Pass,
            summary: format!(
                "File descriptor limits are optimal (soft: {}, hard: {})",
                rlim.rlim_cur, rlim.rlim_max
            ),
            details,
            remediation: None,
        }
    }
}

pub fn get_free_disk_bytes(path: &Path) -> Option<u64> {
    let mut check_path = path.to_path_buf();
    while !check_path.exists() {
        if !check_path.pop() {
            check_path = PathBuf::from("/");
            break;
        }
    }
    let c_path = std::ffi::CString::new(check_path.to_str()?).ok()?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
            Some((stat.f_bavail as u64) * (stat.f_bsize as u64))
        } else {
            None
        }
    }
}

pub fn check_disk_space(paths: &Paths) -> CheckItem {
    let mut details = Vec::new();

    let targets = [
        ("Darling prefix", &paths.darling_prefix),
        ("Cache directory", &paths.cache_dir),
        ("Data directory", &paths.data_dir),
    ];

    let mut min_free_mb = u64::MAX;
    let mut min_name = "";
    let mut min_path = PathBuf::new();

    for (name, path) in &targets {
        let free = get_free_disk_bytes(path).unwrap_or(0);
        let free_mb = free >> 20;
        let free_gb = (free as f64) / 1_000_000_000.0;
        details.push(format!("{}: {:.2} GB free ({})", name, free_gb, path.display()));
        if free_mb < min_free_mb {
            min_free_mb = free_mb;
            min_name = name;
            min_path = (*path).clone();
        }
    }

    if min_free_mb < 300 {
        let remediation = Some(format!(
            "Free up disk space on the volume hosting {}: {}",
            min_name,
            min_path.display()
        ));
        CheckItem {
            category: "Storage",
            name: "Disk Space",
            status: CheckStatus::Fail,
            summary: format!(
                "Critically low disk space on {} (only {} MB free)",
                min_name, min_free_mb
            ),
            details,
            remediation,
        }
    } else if min_free_mb < 1000 {
        let remediation = Some(
            "Ensure at least 2 GB of free disk space is available for Roblox client updates and shader caches."
                .to_string(),
        );
        CheckItem {
            category: "Storage",
            name: "Disk Space",
            status: CheckStatus::Warn,
            summary: format!("Low disk space on {} ({} MB free)", min_name, min_free_mb),
            details,
            remediation,
        }
    } else {
        CheckItem {
            category: "Storage",
            name: "Disk Space",
            status: CheckStatus::Pass,
            summary: format!(
                "Sufficient disk space available across all partitions (min: {:.2} GB free)",
                (min_free_mb as f64) / 1000.0
            ),
            details,
            remediation: None,
        }
    }
}

pub fn check_network() -> CheckItem {
    let mut details = Vec::new();

    let doh_domains = ["roblox.com", "setup.rbxcdn.com"];
    let mut doh_ok = true;
    for domain in &doh_domains {
        let query = build_dns_query(domain);
        let resp = query_doh_sync(&query);
        match resp {
            Some(bytes) if bytes.len() >= 12 => {
                let ans = if bytes.len() >= 8 {
                    u16::from_be_bytes([bytes[6], bytes[7]])
                } else {
                    0
                };
                details.push(format!("DoH resolution for {}: OK ({} answers)", domain, ans));
            }
            _ => {
                doh_ok = false;
                details.push(format!("DoH resolution for {}: Failed / Timeout", domain));
            }
        }
    }

    let endpoints = [
        ("https://roblox.com", "roblox.com"),
        ("https://setup.rbxcdn.com", "setup.rbxcdn.com"),
    ];
    let mut http_ok_count = 0;
    for (url, name) in &endpoints {
        let res = ureq::head(url)
            .timeout(std::time::Duration::from_secs(3))
            .call();
        match res {
            Ok(resp) => {
                http_ok_count += 1;
                details.push(format!("HTTPS to {} (status: {})", name, resp.status()));
            }
            Err(ureq::Error::Status(status, _)) => {
                http_ok_count += 1;
                details.push(format!("HTTPS to {} (connected, status: {})", name, status));
            }
            Err(ureq::Error::Transport(e)) => {
                details.push(format!("HTTPS to {}: failed ({})", name, e));
            }
        }
    }

    if http_ok_count == endpoints.len() && doh_ok {
        CheckItem {
            category: "Network & Connectivity",
            name: "Roblox Endpoints & DoH",
            status: CheckStatus::Pass,
            summary: "All Roblox endpoints reachable via HTTPS and DoH".to_string(),
            details,
            remediation: None,
        }
    } else if http_ok_count > 0 || doh_ok {
        let remediation = Some(
            "Some network probes failed or timed out. If Roblox is restricted in your region, \
             ensure DNS-over-HTTPS (DoH) is active in Settings, or configure a VPN/proxy."
                .to_string(),
        );
        CheckItem {
            category: "Network & Connectivity",
            name: "Roblox Endpoints & DoH",
            status: CheckStatus::Warn,
            summary: "Partial connectivity to Roblox endpoints (some requests timed out)"
                .to_string(),
            details,
            remediation,
        }
    } else {
        let remediation = Some(
            "Cannot establish connection to Roblox endpoints. Check your internet connection, firewall, or ISP censorship.\n\
             Ensure outgoing HTTPS (port 443) and DNS are not blocked."
                .to_string(),
        );
        CheckItem {
            category: "Network & Connectivity",
            name: "Roblox Endpoints & DoH",
            status: CheckStatus::Fail,
            summary: "Failed to connect to Roblox servers (offline or blocked)".to_string(),
            details,
            remediation,
        }
    }
}

fn build_dns_query(domain: &str) -> Vec<u8> {
    let mut query = vec![
        0x12, 0x34, // ID
        0x01, 0x00, // flags
        0x00, 0x01, // Questions: 1
        0x00, 0x00, // Answers: 0
        0x00, 0x00,
        0x00, 0x00,
    ];
    for label in domain.split('.') {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&1u16.to_be_bytes()); // Type A
    query.extend_from_slice(&1u16.to_be_bytes()); // Class IN
    query
}

fn query_doh_sync(query: &[u8]) -> Option<Vec<u8>> {
    let endpoints = [
        "https://dns.google/dns-query",
        "https://cloudflare-dns.com/dns-query",
        "https://dns.quad9.net/dns-query",
    ];
    for url in endpoints {
        let resp = ureq::post(url)
            .set("Content-Type", "application/dns-message")
            .set("Accept", "application/dns-message")
            .timeout(std::time::Duration::from_secs(2))
            .send_bytes(query);

        if let Ok(response) = resp {
            if response.status() == 200 {
                use std::io::Read;
                let mut reader = response.into_reader().take(65536);
                let mut out = Vec::new();
                if std::io::copy(&mut reader, &mut out).is_ok() && out.len() >= 12 {
                    return Some(out);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_system_limits() {
        let item = check_system_limits();
        assert_eq!(item.category, "System Limits");
        assert!(item.status.is_pass() || item.status.is_warn());
    }

    #[test]
    fn test_check_disk_space() {
        let paths = Paths::resolve();
        let item = check_disk_space(&paths);
        assert_eq!(item.category, "Storage");
        assert!(item.status.is_pass() || item.status.is_warn());
    }

    #[test]
    fn test_check_darling() {
        let paths = Paths::resolve();
        let item = check_darling(&paths);
        assert_eq!(item.category, "Darling Container");
    }

    #[test]
    fn test_check_vulkan() {
        let item = check_vulkan();
        assert_eq!(item.category, "Graphics & 3D");
    }

    #[test]
    fn test_check_sound() {
        let item = check_sound();
        assert_eq!(item.category, "Sound Subsystem");
    }

    #[test]
    fn test_doctor_report_generation() {
        let paths = Paths::resolve();
        let report = DoctorReport::run(&paths);
        assert_eq!(report.checks.len(), 6);
        let (p, w, f) = report.summary_counts();
        assert_eq!(p + w + f, 6);
        let dialog_txt = report.to_dialog_text();
        assert!(dialog_txt.contains("Результаты проверки системы Crabblox"));
    }

    #[test]
    fn test_build_dns_query() {
        let q = build_dns_query("roblox.com");
        assert!(q.len() > 12);
        assert_eq!(q[12], 6); // length of 'roblox'
    }
}
