use std::path::{Path, PathBuf};

pub struct Paths {
    pub project_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub darling_prefix: PathBuf,
    pub darling_sysroot: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        // If running from repo, project dir is repo root
        let project_dir = if exe_dir.join("../libMacOBloxShims.m").exists() {
            exe_dir.join("..").canonicalize().unwrap_or(exe_dir)
        } else if Path::new("libMacOBloxShims.m").exists() {
            PathBuf::from(".").canonicalize().unwrap_or_else(|_| PathBuf::from("."))
        } else {
            PathBuf::from("/home/tanukis/MacOBlox")
        };

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data_dir = if std::fs::metadata(&project_dir).map(|m| !m.permissions().readonly()).unwrap_or(false) {
            project_dir.clone()
        } else {
            dirs::data_dir().unwrap_or_else(|| home.join(".local/share")).join("macoblox")
        };

        let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("macoblox");
        let cache_dir = dirs::cache_dir().unwrap_or_else(|| home.join(".cache")).join("macoblox");

        let darling_prefix = std::env::var("DPREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".darling"));

        let darling_sysroot = [
            PathBuf::from("/usr/libexec/darling"),
            PathBuf::from("/usr/local/libexec/darling"),
            PathBuf::from("/app/libexec/darling"),
        ]
        .into_iter()
        .find(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("/usr/libexec/darling"));

        Self {
            project_dir,
            data_dir,
            config_dir,
            cache_dir,
            darling_prefix,
            darling_sysroot,
        }
    }

    pub fn app_bundle(&self) -> PathBuf {
        self.data_dir.join("RobloxPlayer.app")
    }

    pub fn shim_dylib(&self) -> PathBuf {
        if let Ok(shim) = std::env::var("MACOBLOX_PREBUILT_SHIM") {
            PathBuf::from(shim).join("libMacOBloxShims.dylib")
        } else {
            self.data_dir.join("build/libMacOBloxShims.dylib")
        }
    }

    pub fn cookies_plist(&self) -> PathBuf {
        let username = std::env::var("USER").unwrap_or_else(|_| "tanukis".to_string());
        self.darling_prefix
            .join("Users")
            .join(username)
            .join("Library/MacOBlox/Cookies.plist")
    }

    pub fn fast_flags_file(&self) -> PathBuf {
        self.app_bundle()
            .join("Contents/MacOS/ClientSettings/ClientAppSettings.json")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn user_cache_file(&self) -> PathBuf {
        self.cache_dir.join("user.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.data_dir.join("downloads")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.data_dir.join("backups")
    }
}
