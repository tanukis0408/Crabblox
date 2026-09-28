use crate::paths::Paths;
use plist::Value as PlistValue;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub fn installed_version(paths: &Paths) -> Option<String> {
    let plist_path = paths.app_bundle().join("Contents/Info.plist");
    if !plist_path.exists() {
        return None;
    }
    if let Ok(PlistValue::Dictionary(dict)) = PlistValue::from_file(&plist_path) {
        if let Some(ver) = dict.get("CFBundleShortVersionString").and_then(|v| v.as_string()) {
            return Some(ver.to_string());
        }
    }
    None
}

pub fn latest_version() -> anyhow::Result<(String, String)> {
    let url = "https://setup.rbxcdn.com/mac/version";
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(10))
        .call()?
        .into_string()?;
    let upload = resp.trim().to_string();

    // Check version-compat API for human version string
    let compat_url = format!("https://setup.rbxcdn.com/mac/{upload}-version.txt");
    let human_version = if let Ok(resp) = ureq::get(&compat_url).timeout(std::time::Duration::from_secs(5)).call() {
        resp.into_string().unwrap_or_else(|_| upload.clone())
    } else {
        upload.clone()
    };

    Ok((human_version.trim().to_string(), upload))
}

pub fn update_roblox<F>(paths: &Paths, upload: &str, progress: F) -> anyhow::Result<PathBuf>
where
    F: Fn(f32, &str),
{
    let downloads = paths.downloads_dir();
    fs::create_dir_all(&downloads)?;

    let archive = downloads.join(format!("{upload}-RobloxPlayer.zip"));
    let part = downloads.join(format!("{upload}-RobloxPlayer.zip.part"));

    let need_download = if archive.exists() {
        let file = fs::File::open(&archive)?;
        zip::ZipArchive::new(file).is_err()
    } else {
        true
    };

    if need_download {
        let url = format!("https://setup.rbxcdn.com/mac/{upload}-RobloxPlayer.zip");
        progress(0.1, "Connecting to download server...");

        let resp = ureq::get(&url)
            .set("User-Agent", "MacOBlox-rs/Linux")
            .timeout(std::time::Duration::from_secs(60))
            .call()?;

        let total_size: usize = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
        let mut reader = resp.into_reader();
        let mut out = fs::File::create(&part)?;

        let mut buf = [0u8; 65536];
        let mut downloaded: usize = 0;

        loop {
            let n = std::io::Read::read(&mut reader, &mut buf)?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut out, &buf[..n])?;
            downloaded += n;
            if total_size > 0 {
                let frac = 0.1 + (downloaded as f32 / total_size as f32) * 0.8;
                progress(
                    frac,
                    &format!(
                        "Downloading: {} / {} MB",
                        downloaded >> 20,
                        total_size >> 20
                    ),
                );
            }
        }

        if total_size > 0 && downloaded < total_size {
            let _ = fs::remove_file(&part);
            anyhow::bail!("Incomplete download: {downloaded}/{total_size} bytes");
        }

        fs::rename(&part, &archive)?;
    }

    progress(0.92, "Unpacking RobloxPlayer.app...");
    let unpack_dir = tempfile::Builder::new().prefix("unpack-").tempdir_in(&downloads)?;
    let unpack_path = unpack_dir.path();

    // Use system unzip with -o or fallback to zip crate
    let res = std::process::Command::new("unzip")
        .args(["-q", "-o", archive.to_str().unwrap(), "-d", unpack_path.to_str().unwrap()])
        .status();

    let new_bundle = unpack_path.join("RobloxPlayer.app");
    let binary = new_bundle.join("Contents/MacOS/RobloxPlayer");

    if res.map(|s| s.code().unwrap_or(1)).unwrap_or(1) > 1 || !binary.exists() {
        // Fallback to internal zip extractor
        let file = fs::File::open(&archive)?;
        let mut zip = zip::ZipArchive::new(file)?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            let outpath = match file.enclosed_name() {
                Some(p) => unpack_path.join(p),
                None => continue,
            };

            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath)?;
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p)?;
                    }
                }
                let mut outfile = fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
                if let Some(mode) = file.unix_mode() {
                    let _ = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    if !new_bundle.is_dir() {
        anyhow::bail!("Archive does not contain RobloxPlayer.app");
    }

    let old_version = installed_version(paths).unwrap_or_else(|| "unknown".to_string());
    let backups = paths.backups_dir();
    fs::create_dir_all(&backups)?;
    let backup_path = backups.join(format!("RobloxPlayer-{old_version}.app"));

    if backup_path.exists() {
        let _ = fs::remove_dir_all(&backup_path);
    }

    let target_bundle = paths.app_bundle();
    if target_bundle.exists() {
        fs::rename(&target_bundle, &backup_path)?;
    }

    fs::rename(&new_bundle, &target_bundle)?;
    progress(1.0, "Done");

    Ok(backup_path)
}
