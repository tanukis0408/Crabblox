use std::sync::Arc;
use adw::prelude::*;
use gtk4::prelude::*;
use crate::auth;
use crate::paths::Paths;
use crate::runner;
use crate::updater;

pub fn build_play_page(window: &adw::ApplicationWindow, paths: Arc<Paths>) -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let status_page = adw::StatusPage::new();
    status_page.set_icon_name(Some("crabblox"));
    status_page.set_title("Crabblox");
    status_page.set_vexpand(true);

    let ver = updater::installed_version(&paths).unwrap_or_else(|| "не найден".to_string());
    let user_str = auth::signed_in_user(&paths).unwrap_or_else(|| "не авторизован".to_string());
    status_page.set_description(Some(&format!(
        "Roblox {} • Вход выполнен: {}",
        ver, user_str
    )));

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    controls_box.set_halign(gtk4::Align::Center);

    // Play Button
    let play_button = gtk4::Button::with_label("Играть");
    play_button.add_css_class("suggested-action");
    play_button.add_css_class("pill");
    play_button.set_size_request(220, 52);

    #[derive(Debug, Clone)]
    enum PlayState {
        Starting,
        Playing,
        Stopped,
        Failed(String),
    }

    let (tx, rx) = async_channel::bounded::<PlayState>(1);
    let play_btn_rx = play_button.downgrade();
    let win_rx = window.downgrade();

    glib::spawn_future_local(async move {
        while let Ok(state) = rx.recv().await {
            if let Some(btn) = play_btn_rx.upgrade() {
                match state {
                    PlayState::Starting => {
                        btn.set_sensitive(false);
                        btn.set_label("Запуск Roblox…");
                    }
                    PlayState::Playing => {
                        btn.set_sensitive(true);
                        btn.set_label("Остановить игру");
                        btn.remove_css_class("suggested-action");
                        btn.add_css_class("destructive-action");
                    }
                    PlayState::Stopped => {
                        btn.set_sensitive(true);
                        btn.set_label("Играть");
                        btn.remove_css_class("destructive-action");
                        btn.add_css_class("suggested-action");
                    }
                    PlayState::Failed(err) => {
                        btn.set_sensitive(true);
                        btn.set_label("Играть");
                        btn.remove_css_class("destructive-action");
                        btn.add_css_class("suggested-action");

                        if let Some(win) = win_rx.upgrade() {
                            let dialog = adw::AlertDialog::new(
                                Some("Ошибка запуска"),
                                Some(&err),
                            );
                            dialog.add_response("ok", "OK");
                            dialog.present(Some(&win));
                        }
                    }
                }
            }
        }
    });

    let paths_play = paths.clone();
    let play_btn_weak = play_button.downgrade();

    play_button.connect_clicked(move |btn| {
        let p = paths_play.clone();

        // If game is running, clicking the button stops it
        if !runner::roblox_pids().is_empty() || btn.label().as_deref() == Some("Остановить игру") {
            btn.set_sensitive(false);
            btn.set_label("Остановка…");
            std::thread::spawn(|| {
                runner::stop_roblox();
            });
            return;
        }

        btn.set_sensitive(false);
        btn.set_label("Запускаю…");

        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx_clone.send_blocking(PlayState::Failed(format!("Runtime error: {:?}", e)));
                    return;
                }
            };

            let launch_res = rt.block_on(async {
                runner::launch(&p).await
            });

            match launch_res {
                Ok(mut session) => {
                    let _ = tx_clone.send_blocking(PlayState::Starting);
                    let mut seen_roblox = false;
                    let mut last_seen = std::time::Instant::now();

                    loop {
                        if let Ok(Some(status)) = session.child.try_wait() {
                            if !seen_roblox && !status.success() {
                                let log_tail = if let Ok(content) = std::fs::read_to_string(&session.log_path) {
                                    let lines: Vec<&str> = content.lines().collect();
                                    let start = lines.len().saturating_sub(8);
                                    lines[start..].join("\n")
                                } else {
                                    String::new()
                                };
                                let msg = if log_tail.is_empty() {
                                    format!("Roblox завершился с кодом {status}")
                                } else {
                                    format!("Roblox завершился с кодом {status}:\n\n{log_tail}")
                                };
                                let _ = tx_clone.send_blocking(PlayState::Failed(msg));
                            } else {
                                let _ = tx_clone.send_blocking(PlayState::Stopped);
                            }
                            break;
                        }

                        let pids = runner::roblox_pids();
                        if !pids.is_empty() {
                            if !seen_roblox {
                                seen_roblox = true;
                                let _ = tx_clone.send_blocking(PlayState::Playing);
                            }
                            last_seen = std::time::Instant::now();
                        } else if seen_roblox {
                            if last_seen.elapsed().as_secs() >= 4 {
                                let _ = session.child.kill();
                                let _ = tx_clone.send_blocking(PlayState::Stopped);
                                break;
                            }
                        }

                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }

                    session.dns.stop();
                    if let Some(ref r) = session.rpc {
                        r.stop();
                    }
                }
                Err(e) => {
                    let _ = tx_clone.send_blocking(PlayState::Failed(format!("Не удалось запустить:\n{e}")));
                }
            }
        });
    });
    controls_box.append(&play_button);

    // Sign in Button
    let signin_button = gtk4::Button::with_label(if auth::signed_in(&paths) {
        "Сменить аккаунт"
    } else {
        "Войти в Roblox"
    });
    signin_button.add_css_class("pill");
    signin_button.set_size_request(220, -1);

    let paths_auth = paths.clone();
    let status_weak = status_page.downgrade();
    let window_auth_weak = window.downgrade();

    signin_button.connect_clicked(move |_| {
        if let Some(win) = window_auth_weak.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Вход в Roblox"),
                Some("Выберите способ входа: импортировать активную сессию из браузера или ввести куки .ROBLOSECURITY вручную.")
            );

            dialog.add_response("browser", "Из браузера (Chrome / Firefox)");
            dialog.add_response("manual", "Ввести куки вручную");
            dialog.add_response("cancel", "Отмена");
            dialog.set_default_response(Some("browser"));

            let p = paths_auth.clone();
            let st = status_weak.clone();
            let win_resp = win.clone();

            dialog.connect_response(None, move |_, resp| {
                if resp == "browser" {
                    match auth::import_browser_cookies(&p) {
                        Ok(Some((user, browser))) => {
                            if let Some(st_up) = st.upgrade() {
                                let v = updater::installed_version(&p).unwrap_or_else(|| "не найден".to_string());
                                st_up.set_description(Some(&format!(
                                    "Roblox {} • Вход выполнен: {} ({})",
                                    v, user, browser
                                )));
                            }
                        }
                        Ok(None) => {
                            let err_dialog = adw::AlertDialog::new(
                                Some("Сессия не найдена"),
                                Some("Не найдено активных сессий Roblox в браузерах. Сначала авторизуйтесь на roblox.com в Chrome или Firefox.")
                            );
                            err_dialog.add_response("ok", "Понятно");
                            err_dialog.present(Some(&win_resp));
                        }
                        Err(e) => {
                            let err_dialog = adw::AlertDialog::new(
                                Some("Ошибка входа"),
                                Some(&format!("Не удалось получить куки: {}", e))
                            );
                            err_dialog.add_response("ok", "Понятно");
                            err_dialog.present(Some(&win_resp));
                        }
                    }
                }
            });

            dialog.present(Some(&win));
        }
    });
    controls_box.append(&signin_button);

    // Roblox Studio Button
    let studio_button = gtk4::Button::with_label("Roblox Studio");
    studio_button.add_css_class("pill");
    studio_button.set_size_request(220, -1);
    controls_box.append(&studio_button);

    // Community links
    let links_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    links_box.set_halign(gtk4::Align::Center);
    links_box.set_margin_top(8);

    let discord_btn = gtk4::Button::from_icon_name("macoblox-discord-symbolic");
    discord_btn.add_css_class("flat");
    discord_btn.add_css_class("circular");
    discord_btn.set_tooltip_text(Some("Discord"));
    discord_btn.connect_clicked(|_| {
        crate::gui::open_uri("https://discord.gg/bpX9rTttCa");
    });
    links_box.append(&discord_btn);

    let github_btn = gtk4::Button::from_icon_name("macoblox-github-symbolic");
    github_btn.add_css_class("flat");
    github_btn.add_css_class("circular");
    github_btn.set_tooltip_text(Some("GitHub"));
    github_btn.connect_clicked(|_| {
        crate::gui::open_uri("https://github.com/narezy/MacOBlox");
    });
    links_box.append(&github_btn);

    controls_box.append(&links_box);

    status_page.set_child(Some(&controls_box));
    root.append(&status_page);

    // Footer version
    let version_label = gtk4::Label::new(Some("Crabblox 0.13 • Monster Dev"));
    version_label.add_css_class("dim-label");
    version_label.add_css_class("caption");
    version_label.set_margin_bottom(12);
    root.append(&version_label);

    root
}
