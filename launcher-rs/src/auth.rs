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
    array.to_file_xml(&plist_path)?;

    let mut perms = fs::metadata(&plist_path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&plist_path, perms)?;

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
    if let Some(parent) = file.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(file, serde_json::to_string_pretty(accounts)?)?;
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

    // Chromium based browsers
    let chromium_dirs = [
        ("Google Chrome", home.join(".config/google-chrome")),
        ("Chromium", home.join(".config/chromium")),
        ("Brave", home.join(".config/BraveSoftware/Brave-Browser")),
        ("Vivaldi", home.join(".config/vivaldi")),
        ("Microsoft Edge", home.join(".config/microsoft-edge")),
        ("Chrome (Flatpak)", home.join(".var/app/com.google.Chrome/config/google-chrome")),
        ("Brave (Flatpak)", home.join(".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser")),
    ];

    for (bname, base) in chromium_dirs {
        let candidates = [
            base.join("Default/Cookies"),
            base.join("Profile 1/Cookies"),
            base.join("Cookies"),
        ];
        for db in candidates {
            if !db.exists() {
                continue;
            }
            if let Ok(cookie) = extract_chromium_cookie(&db) {
                if let Ok(user) = save_session_cookie(paths, &cookie) {
                    return Ok(Some((user, bname.to_string())));
                }
            }
        }
    }

    // Gecko based browsers
    let gecko_patterns = [
        ("Firefox", home.join(".mozilla/firefox")),
        ("Zen Browser", home.join(".zen")),
        ("Floorp", home.join(".floorp")),
        ("LibreWolf", home.join(".librewolf")),
        ("Firefox (Flatpak)", home.join(".var/app/org.mozilla.firefox/.mozilla/firefox")),
    ];

    for (bname, base) in gecko_patterns {
        if !base.exists() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&base) {
            for entry in entries.flatten() {
                let db = entry.path().join("cookies.sqlite");
                if db.exists() {
                    if let Ok(cookie) = extract_gecko_cookie(&db) {
                        if let Ok(user) = save_session_cookie(paths, &cookie) {
                            return Ok(Some((user, bname.to_string())));
                        }
                    }
                }
            }
        }
    }

    Ok(None)
}

fn extract_chromium_cookie(db_path: &Path) -> anyhow::Result<String> {
    let tmp = tempfile::NamedTempFile::new()?;
    fs::copy(db_path, tmp.path())?;

    let conn = Connection::open(tmp.path())?;
    let mut stmt = conn.prepare(
        "SELECT encrypted_value FROM cookies WHERE host_key LIKE '%roblox%' AND name = '.ROBLOSECURITY' LIMIT 1",
    )?;

    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let enc_bytes: Vec<u8> = row.get(0)?;
        if enc_bytes.starts_with(b"v10") && enc_bytes.len() > 3 {
            // Password logic: try keyring lookup, default to peanuts
            let mut passwords = vec![b"peanuts".to_vec()];
            if let Ok(output) = std::process::Command::new("secret-tool")
                .args(["lookup", "application", "chrome"])
                .output()
            {
                if output.status.success() && !output.stdout.is_empty() {
                    let mut pwd = output.stdout;
                    if pwd.ends_with(b"\n") {
                        pwd.pop();
                    }
                    passwords.insert(0, pwd);
                }
            }

            for pwd in passwords {
                let mut key = [0u8; 16];
                pbkdf2_hmac::<Sha1>(&pwd, b"saltysalt", 1, &mut key);
                let iv = [b' '; 16];

                let mut ciphertext = enc_bytes[3..].to_vec();
                if let Ok(decryptor) = Aes128CbcDec::new_from_slices(&key, &iv) {
                    if let Ok(decrypted) = decryptor.decrypt_padded_mut::<Pkcs7>(&mut ciphertext) {
                        let text = String::from_utf8_lossy(decrypted);
                        if let Some(idx) = text.find("_|WARNING:-DO") {
                            let cookie = &text[idx..];
                            let cleaned = cookie
                                .trim_end_matches(|c: char| c.is_whitespace() || !c.is_ascii_graphic())
                                .to_string();
                            return Ok(cleaned);
                        }
                    }
                }
            }
        }
    }
    anyhow::bail!("Cookie not found in chromium db")
}

fn extract_gecko_cookie(db_path: &Path) -> anyhow::Result<String> {
    let tmp = tempfile::NamedTempFile::new()?;
    fs::copy(db_path, tmp.path())?;

    let conn = Connection::open(tmp.path())?;
    let mut stmt = conn.prepare(
        "SELECT value FROM moz_cookies WHERE host LIKE '%roblox.com' AND name = '.ROBLOSECURITY' LIMIT 1",
    )?;

    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
        let val: String = row.get(0)?;
        if !val.is_empty() {
            return Ok(val.trim().to_string());
        }
    }
    anyhow::bail!("Cookie not found in gecko db")
}
