mod auth;
pub mod discord_rpc;
mod dns;
pub mod doctor;
mod fast_flags;
mod gui;
mod paths;
mod runner;
mod updater;

use clap::{Parser, Subcommand};
use paths::Paths;
use serde_json::Value;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "crabblox")]
#[command(version)]
#[command(about = "Crabblox — High-performance Rust launcher for Roblox on Linux (by Monster Dev)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch the graphical user interface (default)
    Gui,
    /// Launch Roblox directly via CLI
    Run {
        /// Restart Darling container before launching
        #[arg(long)]
        restart_darling: bool,
    },
    /// Sign in to Roblox (from browser or cookie)
    Login {
        /// Import session cookie from installed web browser (Chrome, Firefox, Brave, etc.)
        #[arg(short, long)]
        browser: bool,

        /// Manually specify .ROBLOSECURITY cookie value
        #[arg(short, long)]
        cookie: Option<String>,
    },
    /// Check for Roblox client updates or install latest
    Update,
    /// Show current installation status, account, and paths
    Status,
    /// Run System Doctor health checks (Darling, Vulkan, Audio, Limits, Disk, Network)
    Doctor,
    /// Manage Fast Flags (ClientAppSettings.json)
    Flags {
        /// Set maximum target FPS (e.g. 144, 240, 0 for default)
        #[arg(long)]
        fps: Option<u32>,

        /// Set a custom flag name=value
        #[arg(long)]
        set: Option<String>,

        /// List current fast flags
        #[arg(short, long)]
        list: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    let paths = Arc::new(Paths::resolve());
    let _ = fast_flags::FastFlags::ensure_compatibility_flags(&paths);

    if raw_args.len() > 1 && (raw_args[1].starts_with("roblox-player:") || raw_args[1].starts_with("roblox-studio:")) {
        println!("Crabblox received deep link: {}", raw_args[1]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        return rt.block_on(async {
            run_roblox(paths).await
        });
    }

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Gui) {
        Commands::Gui => {
            gui::run_gui(paths);
        }
        Commands::Run { restart_darling } => {
            if restart_darling {
                println!("Restarting Darling container daemon...");
                runner::restart_darling(&paths.darling_prefix);
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                run_roblox(paths).await
            })?;
        }

        Commands::Login { browser, cookie } => {
            if let Some(cookie_val) = cookie {
                println!("Validating provided cookie...");
                let username = auth::save_session_cookie(&paths, &cookie_val)?;
                println!("Successfully signed in as '{}'!", username);
            } else if browser || cookie.is_none() {
                println!("Searching for active Roblox session in browsers (Chrome, Chromium, Firefox, Brave, Edge, Opera, Zen...)...");
                match auth::import_browser_cookies(&paths)? {
                    Some((user, bname)) => {
                        println!("Successfully imported active session for '{}' from {}!", user, bname);
                    }
                    None => {
                        println!("No active Roblox session found in installed browsers.");
                        println!("Please log in on roblox.com in your browser, or pass --cookie <value>.");
                    }
                }
            }
        }

        Commands::Update => {
            println!("Checking for latest Roblox macOS version...");
            let (human_ver, upload) = updater::latest_version()?;
            let current_ver = updater::installed_version(&paths).unwrap_or_default();
            println!("Latest version: {} ({})", human_ver, upload);
            println!("Installed version: {}", current_ver);

            if current_ver != human_ver {
                println!("Updating...");
                updater::update_roblox(&paths, &upload, |frac, msg| {
                    println!("[{:.0}%] {}", frac * 100.0, msg);
                })?;
                println!("Update complete!");
            } else {
                println!("Already up to date.");
            }
        }

        Commands::Status => {
            println!("=== Crabblox Status (by Monster Dev) ===");
            println!("Project dir: {:?}", paths.project_dir);
            println!("App bundle:  {:?}", paths.app_bundle());
            println!("Darling prefix: {:?}", paths.darling_prefix);
            println!("Shim dylib:  {:?} (exists: {})", paths.shim_dylib(), paths.shim_dylib().exists());
            println!("Installed Roblox version: {}", updater::installed_version(&paths).unwrap_or_else(|| "None".into()));
            println!("Signed in:   {}", auth::signed_in(&paths));
            if let Some(user) = auth::signed_in_user(&paths) {
                println!("User:        {}", user);
            }
        }

        Commands::Doctor => {
            let report = doctor::DoctorReport::run(&paths);
            report.print_cli();
            if report.has_failures() {
                std::process::exit(1);
            }
        }

        Commands::Flags { fps, set, list } => {
            if let Some(fps_val) = fps {
                fast_flags::FastFlags::set_fps_cap(&paths, fps_val)?;
                println!("Set target FPS cap to: {}", fps_val);
            }
            if let Some(set_val) = set.as_ref() {
                if let Some((name, val)) = set_val.split_once('=') {
                    let json_val = serde_json::from_str(val).unwrap_or_else(|_| Value::String(val.to_string()));
                    fast_flags::FastFlags::set_flag(&paths, name, json_val)?;
                    println!("Set flag {} = {}", name, val);
                } else {
                    println!("Format must be: --set FlagName=value");
                }
            }
            if list || (fps.is_none() && set.is_none()) {
                let flags = fast_flags::FastFlags::load(&paths);
                println!("Current Fast Flags ({}):", flags.len());
                for (k, v) in flags {
                    println!("  {} = {}", k, v);
                }
            }
        }
    }

    Ok(())
}

async fn run_roblox(paths: Arc<Paths>) -> anyhow::Result<()> {
    println!("=== Crabblox (by Monster Dev) ===");
    if !auth::signed_in(&paths) {
        println!("No saved Roblox session found. Checking browsers...");
        if let Ok(Some((user, browser))) = auth::import_browser_cookies(&paths) {
            println!("Successfully imported Roblox session for '{}' from {}!", user, browser);
        } else {
            println!("Not signed in! Run 'crabblox login --browser' or use Quick Login in-game.");
        }
    } else if let Some(user) = auth::signed_in_user(&paths) {
        println!("Signed in as: {}", user);
    }

    let ver = updater::installed_version(&paths).unwrap_or_else(|| "not installed".to_string());
    println!("Installed Roblox version: {}", ver);

    if ver == "not installed" {
        println!("Roblox is not installed. Updating to latest version...");
        let (_, upload) = updater::latest_version()?;
        updater::update_roblox(&paths, &upload, |frac, msg| {
            println!("[{:.0}%] {}", frac * 100.0, msg);
        })?;
    }

    println!("Starting local DNS forwarder and launching Roblox...");
    let mut session = runner::launch(&paths).await?;
    println!("Roblox launched successfully! PID: {}", session.child.id());
    println!("Log file: {:?}", session.log_path);

    let status = session.child.wait()?;
    session.dns.stop();
    println!("Roblox exited with status: {}", status);
    if !status.success() {
        runner::scan_crash_diagnostics_with_status(&session.log_path, Some(&status));
    }
    Ok(())
}
