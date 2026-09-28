use std::path::PathBuf;

pub struct Paths {
    pub project_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub darling_prefix: PathBuf,
    #[allow(dead_code)]
    pub darling_sysroot: PathBuf,
}

impl Paths {
    pub fn resolve() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        let data_home = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));

        let candidate_dirs = [
            std::env::var("CRABBLOX_PROJECT_DIR").ok().map(PathBuf::from),
            std::env::var("MACOBLOX_PROJECT_DIR").ok().map(PathBuf::from),
            Some(exe_dir.clone()),
            exe_dir.parent().map(|p| p.to_path_buf()),
            exe_dir.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()),
            Some(PathBuf::from(".")),
            Some(data_home.join("Crabblox")),
            Some(data_home.join("MacOBlox")),
            Some(home.join(".local/share/Crabblox")),
            Some(home.join(".local/share/MacOBlox")),
            Some(home.join("Crabblox")),
            Some(home.join("MacOBlox")),
        ];

        let project_dir = candidate_dirs
            .into_iter()
            .flatten()
            .find(|dir| dir.join("build_debug_shim.sh").exists() || dir.join("libMacOBloxShims.m").exists())
            .map(|dir| dir.canonicalize().unwrap_or(dir))
            .unwrap_or_else(|| data_home.join("Crabblox"));

        let data_dir = if std::fs::metadata(&project_dir).map(|m| !m.permissions().readonly()).unwrap_or(false)
            && project_dir.join("build_debug_shim.sh").exists()
        {
            project_dir.clone()
        } else if data_home.join("Crabblox").exists() {
            data_home.join("Crabblox")
        } else if data_home.join("MacOBlox").exists() {
            data_home.join("MacOBlox")
        } else {
            data_home.join("Crabblox")
        };

        let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("crabblox");
        let cache_dir = dirs::cache_dir().unwrap_or_else(|| home.join(".cache")).join("crabblox");

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
        if self.data_dir.join("RobloxPlayer.app").exists() {
            self.data_dir.join("RobloxPlayer.app")
        } else if self.project_dir.join("RobloxPlayer.app").exists() {
            self.project_dir.join("RobloxPlayer.app")
        } else {
            self.data_dir.join("RobloxPlayer.app")
        }
    }

    pub fn shim_dylib(&self) -> PathBuf {
        if let Ok(shim) = std::env::var("MACOBLOX_PREBUILT_SHIM") {
            PathBuf::from(shim).join("libMacOBloxShims.dylib")
        } else if self.project_dir.join("build/libMacOBloxShims.dylib").exists() {
            self.project_dir.join("build/libMacOBloxShims.dylib")
        } else if self.project_dir.join("prebuilt/libMacOBloxShims.dylib").exists() {
            self.project_dir.join("prebuilt/libMacOBloxShims.dylib")
        } else if self.data_dir.join("build/libMacOBloxShims.dylib").exists() {
            self.data_dir.join("build/libMacOBloxShims.dylib")
        } else if self.data_dir.join("prebuilt/libMacOBloxShims.dylib").exists() {
            self.data_dir.join("prebuilt/libMacOBloxShims.dylib")
        } else {
            self.project_dir.join("build/libMacOBloxShims.dylib")
        }
    }

    pub fn cookies_plist(&self) -> PathBuf {
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        let macoblox = self.darling_prefix
            .join("Users")
            .join(&username)
            .join("Library/MacOBlox/Cookies.plist");
        if macoblox.exists() {
            return macoblox;
        }
        let crabblox = self.darling_prefix
            .join("Users")
            .join(&username)
            .join("Library/Crabblox/Cookies.plist");
        if crabblox.exists() {
            return crabblox;
        }
        macoblox
    }

    pub fn fast_flags_file(&self) -> PathBuf {
        self.app_bundle()
            .join("Contents/MacOS/ClientSettings/ClientAppSettings.json")
    }

    #[allow(dead_code)]
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
