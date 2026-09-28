use crate::paths::Paths;
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use pbkdf2::pbkdf2_hmac;
use plist::Value as PlistValue;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: u64,
    pub name: String,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
}

pub fn signed_in(paths: &Paths) -> bool {
    let plist_path = paths.cookies_plist();
    if !plist_path.exists() {
        return false;
    }
    if let Ok(PlistValue::Array(items)) = PlistValue::from_file(&plist_path) {
        items.iter().any(|item| {
            if let PlistValue::Dictionary(dict) = item {
                dict.get("Name").and_then(|v| v.as_string()) == Some(".ROBLOSECURITY")
                    && dict.get("Value").and_then(|v| v.as_string()).is_some()
            } else {
                false
            }
        })
    } else {
        false
    }
}

pub fn signed_in_user(paths: &Paths) -> Option<String> {
    if !signed_in(paths) {
        return None;
    }
    let cache_file = paths.user_cache_file();
    if let Ok(content) = fs::read_to_string(&cache_file) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
                return Some(name.to_string());
            }
        }
    }
    None
}

pub fn validate_cookie(cookie: &str) -> anyhow::Result<UserInfo> {
    let url = "https://users.roblox.com/v1/users/authenticated";
    let resp: UserInfo = ureq::get(url)
        .set("Cookie", &format!(".ROBLOSECURITY={cookie}"))
        .set("User-Agent", "MacOBlox-rs/Linux")
        .timeout(std::time::Duration::from_secs(10))
        .call()?
        .into_json()?;
    Ok(resp)
}

pub fn save_session_cookie(paths: &Paths, cookie: &str) -> anyhow::Result<String> {
    let raw_cookie = cookie.trim().trim_matches('"').trim_matches('\'');
    let cookie_val = if let Some((_, rest)) = raw_cookie.split_once(".ROBLOSECURITY=") {
        rest.split(';').next().unwrap_or(rest).trim()
    } else {
        raw_cookie
    };

    let user_info = validate_cookie(cookie_val)?;
    let username = user_info.display_name.clone().unwrap_or_else(|| user_info.name.clone());

    let _ = fs::create_dir_all(&paths.cache_dir);
    let _ = fs::write(
        paths.user_cache_file(),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": username,
            "id": user_info.id
        }))?,
    );

    let plist_path = paths.cookies_plist();
    if let Some(parent) = plist_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let expires = std::time::SystemTime::now() + std::time::Duration::from_secs(3650 * 86400);

    let mut dict = plist::Dictionary::new();
    dict.insert("Domain".into(), PlistValue::String(".roblox.com".into()));
    dict.insert("Path".into(), PlistValue::String("/".into()));
    dict.insert("Name".into(), PlistValue::String(".ROBLOSECURITY".into()));
    dict.insert("Value".into(), PlistValue::String(cookie_val.into()));
    dict.insert("Secure".into(), PlistValue::Boolean(true));
    dict.insert("Expires".into(), PlistValue::Date(expires.into()));

    let array = PlistValue::Array(vec![PlistValue::Dictionary(dict)]);

    let username_env = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let macoblox_plist = paths.darling_prefix.join("Users").join(&username_env).join("Library/MacOBlox/Cookies.plist");
    let crabblox_plist = paths.darling_prefix.join("Users").join(&username_env).join("Library/Crabblox/Cookies.plist");
    let primary_plist = paths.cookies_plist();

    for path in [&primary_plist, &macoblox_plist, &crabblox_plist] {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(()) = array.to_file_xml(path) {
            if let Ok(metadata) = fs::metadata(path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o600);
                let _ = fs::set_permissions(path, perms);
            }
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let saved = SavedAccount {
        id: user_info.id,
        username: user_info.name.clone(),
        display_name: username.clone(),
        cookie: cookie_val.to_string(),
        last_used: now,
    };
    let _ = add_or_update_account(paths, saved);

    Ok(username)
}

pub fn sign_out(paths: &Paths) {
    let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let macoblox_plist = paths.darling_prefix.join("Users").join(&username).join("Library/MacOBlox/Cookies.plist");
    let crabblox_plist = paths.darling_prefix.join("Users").join(&username).join("Library/Crabblox/Cookies.plist");
    let _ = fs::remove_file(macoblox_plist);
    let _ = fs::remove_file(crabblox_plist);
    let _ = fs::remove_file(paths.cookies_plist());
    let _ = fs::remove_file(paths.user_cache_file());
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedAccount {
    pub id: u64,
    pub username: String,
    pub display_name: String,
    pub cookie: String,
    pub last_used: u64,
}

pub fn accounts_file(paths: &Paths) -> std::path::PathBuf {
    paths.config_dir.join("accounts.json")
}

pub fn load_accounts(paths: &Paths) -> Vec<SavedAccount> {
    let file = accounts_file(paths);
    if let Ok(content) = fs::read_to_string(&file) {
        if let Ok(accounts) = serde_json::from_str::<Vec<SavedAccount>>(&content) {
            return accounts;
        }
    }
    Vec::new()
}

pub fn save_accounts(paths: &Paths, accounts: &[SavedAccount]) -> anyhow::Result<()> {
    let file = accounts_file(paths);
    let parent = file.parent().ok_or_else(|| anyhow::anyhow!("Invalid accounts path"))?;
    fs::create_dir_all(parent)?;
    let content = serde_json::to_string_pretty(accounts)?;
    let tmp_file = parent.join(format!(".accounts.json.tmp.{}", std::process::id()));
    fs::write(&tmp_file, content)?;
    fs::rename(&tmp_file, &file)?;
    Ok(())
}

pub fn add_or_update_account(paths: &Paths, account: SavedAccount) -> anyhow::Result<()> {
    let mut accounts = load_accounts(paths);
    accounts.retain(|a| a.id != account.id);
    accounts.insert(0, account);
    save_accounts(paths, &accounts)
}

pub fn switch_to_account(paths: &Paths, user_id: u64) -> anyhow::Result<String> {
    let accounts = load_accounts(paths);
    if let Some(account) = accounts.iter().find(|a| a.id == user_id) {
        let user = save_session_cookie(paths, &account.cookie)?;
        let mut updated = accounts;
        if let Some(a) = updated.iter_mut().find(|a| a.id == user_id) {
            a.last_used = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
        }
        let _ = save_accounts(paths, &updated);
        Ok(user)
    } else {
        anyhow::bail!("Аккаунт не найден");
    }
}

pub fn remove_saved_account(paths: &Paths, user_id: u64) -> anyhow::Result<()> {
    let mut accounts = load_accounts(paths);
    accounts.retain(|a| a.id != user_id);
    save_accounts(paths, &accounts)
}

pub fn import_browser_cookies(paths: &Paths) -> anyhow::Result<Option<(String, String)>> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));

    // Chromium-based browsers: native, flatpak, and snap variants
    let chromium_dirs = [
        ("Google Chrome", home.join(".config/google-chrome")),
        ("Chromium", home.join(".config/chromium")),
        ("Brave", home.join(".config/BraveSoftware/Brave-Browser")),
        ("Vivaldi", home.join(".config/vivaldi")),
        ("Microsoft Edge", home.join(".config/microsoft-edge")),
        ("Opera", home.join(".config/opera")),
        ("Opera GX", home.join(".config/opera-gx")),
        ("Yandex Browser", home.join(".config/yandex-browser")),
        ("Thorium", home.join(".config/thorium")),
        ("Chromium (Snap)", home.join("snap/chromium/common/chromium")),
        ("Chromium (Snap Alt)", home.join("snap/chromium/current/.config/chromium")),
        ("Chrome (Flatpak)", home.join(".var/app/com.google.Chrome/config/google-chrome")),
        ("Chromium (Flatpak)", home.join(".var/app/org.chromium.Chromium/config/chromium")),
        ("Brave (Flatpak)", home.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser")),
        ("Edge (Flatpak)", home.join(".var/app/com.microsoft.Edge/config/microsoft-edge")),
        ("Vivaldi (Flatpak)", home.join(".var/app/com.vivaldi.Vivaldi/config/vivaldi")),
        ("Opera (Flatpak)", home.join(".var/app/com.opera.Opera/config/opera")),
    ];

    let passwords = get_chromium_passwords();

    for (bname, base) in chromium_dirs {
        for db in find_chromium_cookie_files(&base) {
            let candidates = extract_chromium_cookies(&db, &passwords);
            for cookie in candidates {
                if let Ok(user) = save_session_cookie(paths, &cookie) {
                    return Ok(Some((user, bname.to_string())));
                }
            }
        }
    }

    // Gecko-based browsers (Firefox, Zen, Floorp, LibreWolf, Waterfox, etc.)
    let gecko_patterns = [
        ("Firefox", home.join(".mozilla/firefox")),
        ("Firefox (Snap)", home.join("snap/firefox/common/.mozilla/firefox")),
        ("Firefox (Flatpak)", home.join(".var/app/org.mozilla.firefox/.mozilla/firefox")),
        ("Zen Browser", home.join(".zen")),
        ("Zen Browser (Flatpak)", home.join(".var/app/app.zen_browser.zen/.zen")),
        ("Floorp", home.join(".floorp")),
        ("Floorp (Flatpak)", home.join(".var/app/one.ablaze.floorp/.floorp")),
        ("LibreWolf", home.join(".librewolf")),
        ("LibreWolf (Flatpak)", home.join(".var/app/io.gitlab.librewolf-community/.librewolf")),
        ("Waterfox", home.join(".waterfox")),
    ];

    for (bname, base) in gecko_patterns {
        for db in find_gecko_cookie_files(&base) {
            let candidates = extract_gecko_cookies(&db);
            for cookie in candidates {
                if let Ok(user) = save_session_cookie(paths, &cookie) {
                    return Ok(Some((user, bname.to_string())));
                }
            }
        }
    }

    Ok(None)
}

fn run_command_with_timeout(cmd: &str, args: &[&str], timeout_duration: std::time::Duration) -> Option<Vec<u8>> {
    let mut child = std::process::Command::new(cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    let mut stdout = Vec::new();
                    if let Some(mut out) = child.stdout.take() {
                        use std::io::Read;
                        let _ = out.read_to_end(&mut stdout);
                    }
                    return Some(stdout);
                }
                return None;
            }
            Ok(None) => {
                if start.elapsed() >= timeout_duration {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
            Err(_) => {
                let _ = child.kill();
                return None;
            }
        }
    }
}

fn get_chromium_passwords() -> Vec<Vec<u8>> {
    let mut passwords: Vec<Vec<u8>> = Vec::new();

    // 1. Try secret-tool lookup for all known Chromium derivatives on Linux with safe timeout
    let app_names = [
        "chrome",
        "google-chrome",
        "chromium",
        "brave",
        "microsoft-edge",
        "edge",
        "opera",
        "opera-gx",
        "vivaldi",
        "yandex-browser",
        "thorium",
    ];

    for app in app_names {
        if let Some(mut pwd) = run_command_with_timeout(
            "secret-tool",
            &["lookup", "application", app],
            std::time::Duration::from_millis(500),
        ) {
            while pwd.ends_with(b"\n") || pwd.ends_with(b"\r") {
                pwd.pop();
            }
            if !pwd.is_empty() && !passwords.contains(&pwd) {
                passwords.push(pwd);
            }
        }
    }

    // 2. Try kwallet-query for KDE Plasma users with safe timeout
    let kwallet_pairs = [
        ("Chrome Keys", "Chrome Safe Storage"),
        ("Chromium Keys", "Chromium Safe Storage"),
        ("Brave Keys", "Brave Safe Storage"),
        ("Edge Keys", "Edge Safe Storage"),
        ("Opera Keys", "Opera Safe Storage"),
        ("Vivaldi Keys", "Vivaldi Safe Storage"),
    ];
    let wallets = ["kdewallet", "kdewallet5", "kdewallet6"];
    for (folder, entry) in kwallet_pairs {
        for wallet in wallets {
            if let Some(mut pwd) = run_command_with_timeout(
                "kwallet-query",
                &["-r", entry, "-f", folder, wallet],
                std::time::Duration::from_millis(300),
            ) {
                while pwd.ends_with(b"\n") || pwd.ends_with(b"\r") {
                    pwd.pop();
                }
                if !pwd.is_empty() && !passwords.contains(&pwd) {
                    passwords.push(pwd);
                }
            }
        }
    }

    // 3. Fallback passwords (Linux basic storage / peanuts)
    if !passwords.contains(&b"peanuts".to_vec()) {
        passwords.push(b"peanuts".to_vec());
    }
    if !passwords.contains(&b"".to_vec()) {
        passwords.push(b"".to_vec());
    }

    passwords
}

fn find_chromium_cookie_files(base: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if !base.exists() {
        return files;
    }

    // Root paths
    for candidate in ["Cookies", "Network/Cookies"] {
        let p = base.join(candidate);
        if p.exists() {
            files.push(p);
        }
    }

    // Subdirectories (Default, Profile 1, Profile 2, etc.)
    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                for candidate in ["Cookies", "Network/Cookies"] {
                    let p = path.join(candidate);
                    if p.exists() && !files.contains(&p) {
                        files.push(p);
                    }
                }
            }
        }
    }

    files
}

fn find_gecko_cookie_files(base: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if !base.exists() {
        return files;
    }

    let direct = base.join("cookies.sqlite");
    if direct.exists() {
        files.push(direct);
    }

    if let Ok(entries) = fs::read_dir(base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let db = path.join("cookies.sqlite");
                if db.exists() && !files.contains(&db) {
                    files.push(db);
                }
            }
        }
    }

    files
}

fn extract_chromium_cookies(db_path: &Path, passwords: &[Vec<u8>]) -> Vec<String> {
    let mut results = Vec::new();
    let temp_dir = match tempfile::tempdir() {
        Ok(t) => t,
        Err(_) => return results,
    };
    let tmp_db = temp_dir.path().join("Cookies");
    if fs::copy(db_path, &tmp_db).is_err() {
        return results;
    }

    // Crucial: copy journal, WAL, and SHM files to ensure hot journal recovery works
    let base_str = db_path.display().to_string();
    for suffix in ["-wal", "-shm", "-journal"] {
        let aux = std::path::PathBuf::from(format!("{}{}", base_str, suffix));
        if aux.exists() {
            let _ = fs::copy(&aux, temp_dir.path().join(format!("Cookies{}", suffix)));
        }
    }

    let conn = match Connection::open_with_flags(&tmp_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(_) => return results,
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(1500));

    let query_ordered = "SELECT value, encrypted_value FROM cookies WHERE (host_key LIKE '%roblox.com%' OR host_key LIKE '%roblox%') AND name = '.ROBLOSECURITY' ORDER BY last_access_utc DESC, creation_utc DESC";
    let query_fallback = "SELECT value, encrypted_value FROM cookies WHERE (host_key LIKE '%roblox.com%' OR host_key LIKE '%roblox%') AND name = '.ROBLOSECURITY'";

    let mut stmt = match conn.prepare(query_ordered) {
        Ok(s) => s,
        Err(_) => match conn.prepare(query_fallback) {
            Ok(s) => s,
            Err(_) => return results,
        },
    };

    let mut rows = match stmt.query([]) {
        Ok(r) => r,
        Err(_) => return results,
    };

    while let Ok(Some(row)) = rows.next() {
        // Plain text value check
        if let Ok(plain_val) = row.get::<_, String>(0) {
            let trimmed = plain_val.trim();
            if !trimmed.is_empty() {
                let cleaned = trimmed
                    .trim_end_matches(|c: char| c.is_whitespace() || !c.is_ascii_graphic())
                    .to_string();
                if (cleaned.contains("_|WARNING:-DO") || cleaned.len() > 50) && !results.contains(&cleaned) {
                    results.push(cleaned);
                }
            }
        }

        // Encrypted value check
        if let Ok(enc_bytes) = row.get::<_, Vec<u8>>(1) {
            if enc_bytes.starts_with(b"v10") && enc_bytes.len() > 3 {
                for pwd in passwords {
                    let mut key = [0u8; 16];
                    pbkdf2_hmac::<Sha1>(pwd, b"saltysalt", 1, &mut key);
                    let iv = [b' '; 16];

                    let mut ciphertext = enc_bytes[3..].to_vec();
                    if let Ok(decryptor) = Aes128CbcDec::new_from_slices(&key, &iv) {
                        if let Ok(decrypted) = decryptor.decrypt_padded_mut::<Pkcs7>(&mut ciphertext) {
                            let text = String::from_utf8_lossy(decrypted);
                            let cookie = if let Some(idx) = text.find("_|WARNING:-DO") {
                                &text[idx..]
                            } else {
                                text.trim()
                            };
                            let cleaned = cookie
                                .trim_end_matches(|c: char| c.is_whitespace() || !c.is_ascii_graphic())
                                .to_string();
                            if !cleaned.is_empty() && !results.contains(&cleaned) {
                                results.push(cleaned);
                            }
                        }
                    }
                }
            }
        }
    }

    results
}

fn extract_gecko_cookies(db_path: &Path) -> Vec<String> {
    let mut results = Vec::new();
    let temp_dir = match tempfile::tempdir() {
        Ok(t) => t,
        Err(_) => return results,
    };
    let tmp_db = temp_dir.path().join("cookies.sqlite");
    if fs::copy(db_path, &tmp_db).is_err() {
        return results;
    }

    let base_str = db_path.display().to_string();
    for suffix in ["-wal", "-shm", "-journal"] {
        let aux = std::path::PathBuf::from(format!("{}{}", base_str, suffix));
        if aux.exists() {
            let _ = fs::copy(&aux, temp_dir.path().join(format!("cookies.sqlite{}", suffix)));
        }
    }

    let conn = match Connection::open_with_flags(&tmp_db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(_) => return results,
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(1500));

    let query_ordered = "SELECT value FROM moz_cookies WHERE (host LIKE '%roblox.com%' OR host LIKE '%roblox%') AND name = '.ROBLOSECURITY' ORDER BY lastAccessed DESC, creationTime DESC";
    let query_fallback = "SELECT value FROM moz_cookies WHERE (host LIKE '%roblox.com%' OR host LIKE '%roblox%') AND name = '.ROBLOSECURITY'";

    let mut stmt = match conn.prepare(query_ordered) {
        Ok(s) => s,
        Err(_) => match conn.prepare(query_fallback) {
            Ok(s) => s,
            Err(_) => return results,
        },
    };

    let mut rows = match stmt.query([]) {
        Ok(r) => r,
        Err(_) => return results,
    };

    while let Ok(Some(row)) = rows.next() {
        if let Ok(val) = row.get::<_, String>(0) {
            let cleaned = val
                .trim()
                .trim_end_matches(|c: char| c.is_whitespace() || !c.is_ascii_graphic())
                .to_string();
            if !cleaned.is_empty() && !results.contains(&cleaned) {
                results.push(cleaned);
            }
        }
    }

    results
}
