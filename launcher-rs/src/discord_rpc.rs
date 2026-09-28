use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const CLIENT_ID: &str = "1554083488968216638"; // User Crabblox Discord Application ID

pub struct DiscordRpc {
    stop_signal: Arc<AtomicBool>,
}

impl DiscordRpc {
    pub fn start(details: String, state: String) -> Self {
        let stop_signal = Arc::new(AtomicBool::new(false));
        let stop_clone = stop_signal.clone();

        std::thread::spawn(move || {
            let stream = match find_and_connect_socket() {
                Some(s) => s,
                None => return, // Discord not running, silently return
            };

            let _ = run_rpc_loop(stream, details, state, stop_clone);
        });

        Self { stop_signal }
    }

    pub fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
    }
}

fn find_and_connect_socket() -> Option<UnixStream> {
    let mut candidates = Vec::new();

    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        for i in 0..10 {
            candidates.push(PathBuf::from(&runtime).join(format!("discord-ipc-{}", i)));
            candidates.push(PathBuf::from(&runtime).join(format!("app/com.discordapp.Discord/discord-ipc-{}", i)));
            candidates.push(PathBuf::from(&runtime).join(format!("app/com.discordapp.DiscordCanary/discord-ipc-{}", i)));
        }
    }

    let uid = unsafe { libc::getuid() };
    for i in 0..10 {
        candidates.push(PathBuf::from(format!("/run/user/{}/discord-ipc-{}", uid, i)));
        candidates.push(PathBuf::from(format!("/run/user/{}/app/com.discordapp.Discord/discord-ipc-{}", uid, i)));
        candidates.push(PathBuf::from(format!("/run/user/{}/app/com.discordapp.DiscordCanary/discord-ipc-{}", uid, i)));
        candidates.push(PathBuf::from(format!("/tmp/discord-ipc-{}", i)));
    }

    if let Some(home) = dirs::home_dir() {
        for i in 0..5 {
            candidates.push(home.join(format!(".var/app/com.discordapp.Discord/data/discord-ipc-{}", i)));
            candidates.push(home.join(format!(".var/app/com.discordapp.DiscordCanary/data/discord-ipc-{}", i)));
        }
    }

    for path in candidates {
        if path.exists() {
            if let Ok(stream) = UnixStream::connect(&path) {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
                return Some(stream);
            }
        }
    }
    None
}

fn send_packet(stream: &mut UnixStream, opcode: u32, payload: &str) -> std::io::Result<()> {
    let len = payload.len() as u32;
    stream.write_all(&opcode.to_le_bytes())?;
    stream.write_all(&len.to_le_bytes())?;
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;
    Ok(())
}

fn run_rpc_loop(
    mut stream: UnixStream,
    details: String,
    state: String,
    stop_signal: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let client_id = std::env::var("CRABBLOX_DISCORD_CLIENT_ID")
        .unwrap_or_else(|_| CLIENT_ID.to_string());

    // 1. Handshake (Opcode 0)
    let handshake = serde_json::json!({
        "v": 1,
        "client_id": client_id
    });
    send_packet(&mut stream, 0, &handshake.to_string())?;

    // Read handshake response
    let mut header = [0u8; 8];
    if stream.read_exact(&mut header).is_ok() {
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let mut buf = vec![0u8; len];
        let _ = stream.read_exact(&mut buf);
    }

    // 2. Set Activity (Opcode 1)
    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Probe candidate asset keys to see which one was registered in Discord Dev Portal
    let candidate_keys = ["crab", "crabblox", "logo", "icon"];
    let mut chosen_key = "crab";
    let mut found = false;

    for &key in &candidate_keys {
        let activity = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": {
                    "details": details,
                    "state": state,
                    "timestamps": {
                        "start": start_time
                    },
                    "assets": {
                        "large_image": key,
                        "large_text": "Crabblox by Monster Dev"
                    },
                    "buttons": [
                        {
                            "label": "Crabblox (Monster Dev)",
                            "url": "https://github.com/tanukis0408/Crabblox"
                        }
                    ]
                }
            },
            "nonce": key
        });

        if send_packet(&mut stream, 1, &activity.to_string()).is_ok() {
            let mut resp_header = [0u8; 8];
            if stream.read_exact(&mut resp_header).is_ok() {
                let len = u32::from_le_bytes([resp_header[4], resp_header[5], resp_header[6], resp_header[7]]) as usize;
                let mut buf = vec![0u8; len];
                if stream.read_exact(&mut buf).is_ok() {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&buf) {
                        if let Some(assets) = val.get("data").and_then(|d| d.get("assets")) {
                            if assets.get("large_image").is_some() {
                                chosen_key = key;
                                found = true;
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    if !found {
        let fallback_act = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {
                "pid": std::process::id(),
                "activity": {
                    "details": details,
                    "state": state,
                    "timestamps": {
                        "start": start_time
                    },
                    "assets": {
                        "large_image": chosen_key,
                        "large_text": "Crabblox by Monster Dev"
                    },
                    "buttons": [
                        {
                            "label": "Crabblox (Monster Dev)",
                            "url": "https://github.com/tanukis0408/Crabblox"
                        }
                    ]
                }
            },
            "nonce": "fallback"
        });
        let _ = send_packet(&mut stream, 1, &fallback_act.to_string());
    }

    // 3. Keep-alive loop while Roblox runs
    while !stop_signal.load(Ordering::SeqCst) {
        let mut header = [0u8; 8];
        match stream.read_exact(&mut header) {
            Ok(()) => {
                let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
                let mut buf = vec![0u8; len];
                let _ = stream.read_exact(&mut buf);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                // Timeout is normal on non-blocking / timed stream
            }
            Err(_) => {
                // Discord closed connection or socket error
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    // Clear Activity on exit
    let clear = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": std::process::id(),
            "activity": null
        },
        "nonce": "2"
    });
    let _ = send_packet(&mut stream, 1, &clear.to_string());

    Ok(())
}
