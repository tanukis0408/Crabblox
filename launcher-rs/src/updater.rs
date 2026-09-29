use crate::paths::Paths;
use plist::Value as PlistValue;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub fn installed_version(paths: &Paths) -> Option<String> {
    let bundle = paths.app_bundle();
    let binary = bundle.join("Contents/MacOS/RobloxPlayer");
    if !binary.exists() {
        return None;
    }
    let plist_path = bundle.join("Contents/Info.plist");
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
    let url = "https://clientsettingscdn.roblox.com/v2/client-version/MacPlayer";
    let resp: serde_json::Value = ureq::get(url)
        .set("User-Agent", "Crabblox/Linux")
        .timeout(std::time::Duration::from_secs(10))
        .call()?
        .into_json()?;

    let version = resp["version"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'version' in response: {:?}", resp))?
        .to_string();

    let upload = resp["clientVersionUpload"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing 'clientVersionUpload' in response: {:?}", resp))?
        .to_string();

    Ok((version, upload))
}

pub fn update_roblox<F>(paths: &Paths, upload: &str, progress: F) -> anyhow::Result<PathBuf>
where
    F: Fn(f32, &str),
{
    let downloads = paths.downloads_dir();
    fs::create_dir_all(&downloads)?;

    // Disk space pre-flight check: ensure at least 1.5 GB is available
    crate::runner::check_disk_space(&downloads, 1500 * 1024 * 1024)?;

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
        let mut attempts = 0;
        let max_attempts = 4;
        loop {
            attempts += 1;
            let existing_offset = if part.exists() {
                fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };

            let is_resuming = existing_offset > 0;
            if is_resuming {
                progress(
                    0.05,
                    &format!(
                        "Возобновление загрузки с {} МБ (попытка {attempts}/{max_attempts})...",
                        existing_offset >> 20
                    ),
                );
            } else {
                progress(
                    0.05,
                    &format!("Подключение к серверу загрузки (попытка {attempts}/{max_attempts})..."),
                );
            }

            let res: Result<(), anyhow::Error> = (|| {
                let mut req = ureq::get(&url)
                    .set("User-Agent", "Crabblox/Linux")
                    .timeout(std::time::Duration::from_secs(60));

                if existing_offset > 0 {
                    req = req.set("Range", &format!("bytes={existing_offset}-"));
                }

                let resp = match req.call() {
                    Ok(r) => r,
                    Err(ureq::Error::Status(416, _)) => {
                        // Range Not Satisfiable: cached file might be complete or corrupted, re-download
                        let _ = fs::remove_file(&part);
                        ureq::get(&url)
                            .set("User-Agent", "Crabblox/Linux")
                            .timeout(std::time::Duration::from_secs(60))
                            .call()?
                    }
                    Err(e) => return Err(e.into()),
                };

                let is_partial = resp.status() == 206;
                let (total_size, mut out) = if is_partial && existing_offset > 0 {
                    let content_range = resp.header("Content-Range");
                    let total = content_range
                        .and_then(|cr| cr.split('/').last())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    let file = fs::OpenOptions::new().append(true).open(&part)?;
                    (total, file)
                } else {
                    let total: u64 = resp.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
                    let file = fs::File::create(&part)?;
                    (total, file)
                };

                let mut reader = resp.into_reader();
                let mut buf = [0u8; 65536];
                let mut downloaded = if is_partial { existing_offset } else { 0 };

                loop {
                    let n = std::io::Read::read(&mut reader, &mut buf)?;
                    if n == 0 {
                        break;
                    }
                    std::io::Write::write_all(&mut out, &buf[..n])?;
                    downloaded += n as u64;
                    if total_size > 0 {
                        let frac = 0.05 + (downloaded as f32 / total_size as f32) * 0.85;
                        progress(
                            frac,
                            &format!(
                                "Загрузка: {} / {} МБ ({:.0}%)",
                                downloaded >> 20,
                                total_size >> 20,
                                (downloaded as f32 / total_size as f32) * 100.0
                            ),
                        );
                    }
                }

                if total_size > 0 && downloaded < total_size {
                    anyhow::bail!("Неполная загрузка: {downloaded}/{total_size} байт");
                }
                Ok(())
            })();

            match res {
                Ok(()) => {
                    // Verify zip integrity before renaming
                    let file = fs::File::open(&part)?;
                    if zip::ZipArchive::new(file).is_err() {
                        let _ = fs::remove_file(&part);
                        anyhow::bail!("Загруженный архив повреждён, требуется повторная загрузка.");
                    }
                    fs::rename(&part, &archive)?;
                    break;
                }
                Err(e) => {
                    if attempts >= max_attempts {
                        let _ = fs::remove_file(&part);
                        return Err(e);
                    }
                    progress(0.05, &format!("Ошибка загрузки ({e}), повтор через 2 сек..."));
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        }
    }

    progress(0.92, "Распаковка RobloxPlayer.app...");
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
            } else if let Some(mode) = file.unix_mode() {
                if (mode & 0o170000) == 0o120000 {
                    // Symlink entry
                    use std::io::Read;
                    let mut link_target = String::new();
                    file.read_to_string(&mut link_target)?;
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            fs::create_dir_all(p)?;
                        }
                    }
                    let _ = fs::remove_file(&outpath);
                    #[cfg(unix)]
                    let _ = std::os::unix::fs::symlink(&link_target, &outpath);
                } else {
                    if let Some(p) = outpath.parent() {
                        if !p.exists() {
                            fs::create_dir_all(p)?;
                        }
                    }
                    let mut outfile = fs::File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;
                    let _ = fs::set_permissions(&outpath, fs::Permissions::from_mode(mode));
                }
            } else {
                if let Some(p) = outpath.parent() {
                    if !p.exists() {
                        fs::create_dir_all(p)?;
                    }
                }
                let mut outfile = fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }
    }

    let binary = new_bundle.join("Contents/MacOS/RobloxPlayer");
    if binary.exists() {
        let _ = fs::set_permissions(&binary, fs::Permissions::from_mode(0o755));
    }
    let crash_handler = new_bundle.join("Contents/MacOS/RobloxCrashHandler");
    if crash_handler.exists() {
        let _ = fs::set_permissions(&crash_handler, fs::Permissions::from_mode(0o755));
    }

    if !new_bundle.is_dir() || !binary.exists() {
        anyhow::bail!("Archive does not contain a valid RobloxPlayer.app");
    }

    let flags = crate::fast_flags::FastFlags::load(paths);
    let old_version = installed_version(paths).unwrap_or_else(|| "unknown".to_string());
    let backups = paths.backups_dir();
    fs::create_dir_all(&backups)?;
    let backup_path = backups.join(format!("RobloxPlayer-{old_version}.app"));

    if backup_path.exists() {
        let _ = fs::remove_dir_all(&backup_path);
    }

    let target_bundle = paths.app_bundle();
    if let Some(parent) = target_bundle.parent() {
        fs::create_dir_all(parent)?;
    }

    if target_bundle.exists() {
        if fs::rename(&target_bundle, &backup_path).is_err() {
            let _ = std::process::Command::new("cp")
                .args(["-a", target_bundle.to_str().unwrap(), backup_path.to_str().unwrap()])
                .status();
            let _ = fs::remove_dir_all(&target_bundle);
        }
    }
    let _ = fs::remove_dir_all(&target_bundle);

    if fs::rename(&new_bundle, &target_bundle).is_err() {
        let status = std::process::Command::new("cp")
            .args(["-a", new_bundle.to_str().unwrap(), target_bundle.to_str().unwrap()])
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            anyhow::bail!("Failed to place new RobloxPlayer.app at {:?}", target_bundle);
        }
    }

    let target_binary = target_bundle.join("Contents/MacOS/RobloxPlayer");
    if target_binary.exists() {
        let _ = fs::set_permissions(&target_binary, fs::Permissions::from_mode(0o755));
    }
    let target_crash = target_bundle.join("Contents/MacOS/RobloxCrashHandler");
    if target_crash.exists() {
        let _ = fs::set_permissions(&target_crash, fs::Permissions::from_mode(0o755));
    }

    if !flags.is_empty() {
        let _ = crate::fast_flags::FastFlags::save(paths, &flags);
    }

    // Keep only the 2 latest backups to prevent disk overflow
    rotate_backups(&backups, 2);

    progress(1.0, "Готово");

    Ok(backup_path)
}

pub fn rotate_backups(backups_dir: &std::path::Path, max_keep: usize) {
    if let Ok(entries) = fs::read_dir(backups_dir) {
        let mut list: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    list.push((mtime, path));
                }
            }
        }
        list.sort_by(|a, b| b.0.cmp(&a.0));
        if list.len() > max_keep {
            for (_, old_path) in list.into_iter().skip(max_keep) {
                if old_path.is_dir() {
                    let _ = fs::remove_dir_all(&old_path);
                } else {
                    let _ = fs::remove_file(&old_path);
                }
            }
        }
    }
}
