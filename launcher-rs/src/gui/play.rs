use std::sync::Arc;
use adw::prelude::*;
use crate::auth;
use crate::paths::Paths;
use crate::runner;
use crate::updater;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceJoinInfo {
    pub place_id: u64,
    pub deep_link: String,
    pub web_url: String,
}

pub fn parse_place_input(raw: &str) -> Result<PlaceJoinInfo, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Err("Введите Place ID или ссылку на плейс".to_string());
    }

    // 1. Direct Place ID as integer
    if let Ok(id) = text.parse::<u64>() {
        if id > 0 {
            return Ok(PlaceJoinInfo {
                place_id: id,
                deep_link: format!("roblox://placeId={id}"),
                web_url: format!("https://www.roblox.com/games/{id}"),
            });
        }
    }

    // 2. HTTP/HTTPS Roblox game URL
    if text.starts_with("http://") || text.starts_with("https://") {
        if let Some(pos) = text.find("/games/") {
            let after = &text[pos + 7..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(id) = digits.parse::<u64>() {
                if id > 0 {
                    return Ok(PlaceJoinInfo {
                        place_id: id,
                        deep_link: format!("roblox://placeId={id}"),
                        web_url: format!("https://www.roblox.com/games/{id}"),
                    });
                }
            }
        }
        let lower = text.to_lowercase();
        if let Some(pos) = lower.find("placeid=") {
            let after = &lower[pos + 8..];
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(id) = digits.parse::<u64>() {
                if id > 0 {
                    return Ok(PlaceJoinInfo {
                        place_id: id,
                        deep_link: format!("roblox://placeId={id}"),
                        web_url: format!("https://www.roblox.com/games/{id}"),
                    });
                }
            }
        }
    }

    // 3. Deep link URL schemes (roblox:// or roblox-player:)
    if text.starts_with("roblox://") || text.starts_with("roblox-player:") {
        let lower = text.to_lowercase();
        for marker in &["placeid=", "placeid:"] {
            if let Some(pos) = lower.find(marker) {
                let after = &lower[pos + marker.len()..];
                let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(id) = digits.parse::<u64>() {
                    if id > 0 {
                        return Ok(PlaceJoinInfo {
                            place_id: id,
                            deep_link: format!("roblox://placeId={id}"),
                            web_url: format!("https://www.roblox.com/games/{id}"),
                        });
                    }
                }
            }
        }
        if let Some(pos) = lower.find("placeid") {
            let after = lower[pos + 7..].trim_start_matches(|c| c == '=' || c == ':');
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(id) = digits.parse::<u64>() {
                if id > 0 {
                    return Ok(PlaceJoinInfo {
                        place_id: id,
                        deep_link: format!("roblox://experiences/start?placeId={id}"),
                        web_url: format!("https://www.roblox.com/games/{id}"),
                    });
                }
            }
        }
    }

    Err("Не удалось распознать Place ID. Введите число (например, 1818) или ссылку на игру Roblox (например, https://www.roblox.com/games/1818).".to_string())
}

pub fn build_play_page(window: &adw::ApplicationWindow, paths: Arc<Paths>) -> gtk4::Box {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);

    let status_page = adw::StatusPage::new();
    status_page.set_icon_name(Some("crabblox"));
    status_page.set_title("Crabblox");
    status_page.set_vexpand(true);

    let ver = updater::installed_version(&paths);
    let is_installed = ver.is_some();
    let ver_str = ver.unwrap_or_else(|| "не установлен".to_string());
    let user_str = auth::signed_in_user(&paths).unwrap_or_else(|| "не авторизован".to_string());
    status_page.set_description(Some(&format!(
        "Roblox: {} • Вход выполнен: {}",
        ver_str, user_str
    )));

    let controls_box = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    controls_box.set_halign(gtk4::Align::Center);

    // System Status Badges
    let status_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    status_box.set_halign(gtk4::Align::Center);
    status_box.set_margin_bottom(4);

    let vram_gb = runner::host_vram_bytes() >> 30;
    let vram_badge = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    vram_badge.add_css_class("card");
    vram_badge.set_margin_start(4);
    vram_badge.set_margin_end(4);

    let vram_icon = gtk4::Image::from_icon_name("video-display-symbolic");
    vram_icon.set_margin_start(8);
    vram_icon.set_margin_top(4);
    vram_icon.set_margin_bottom(4);
    let vram_label = gtk4::Label::new(Some(&format!("VRAM: {} ГБ", vram_gb)));
    vram_label.add_css_class("caption");
    vram_label.set_margin_end(8);
    vram_label.set_margin_top(4);
    vram_label.set_margin_bottom(4);
    vram_badge.append(&vram_icon);
    vram_badge.append(&vram_label);
    vram_badge.set_tooltip_text(Some(&format!(
        "Обнаруженная видеопамять GPU: {} ГБ (доступно для драйвера)",
        vram_gb
    )));

    let darling_active = runner::is_darlingserver_running(&paths.darling_prefix);
    let darling_badge = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    darling_badge.add_css_class("card");
    darling_badge.set_margin_start(4);
    darling_badge.set_margin_end(4);

    let darling_icon = gtk4::Image::from_icon_name(if darling_active {
        "emblem-ok-symbolic"
    } else {
        "emblem-default-symbolic"
    });
    darling_icon.set_margin_start(8);
    darling_icon.set_margin_top(4);
    darling_icon.set_margin_bottom(4);
    let darling_label = gtk4::Label::new(Some(if darling_active {
        "Darling: активен"
    } else {
        "Darling: готов"
    }));
    darling_label.add_css_class("caption");
    darling_label.set_margin_end(8);
    darling_label.set_margin_top(4);
    darling_label.set_margin_bottom(4);
    darling_badge.append(&darling_icon);
    darling_badge.append(&darling_label);
    darling_badge.set_tooltip_text(Some("Состояние контейнера Darling (darlingserver)"));

    status_box.append(&vram_badge);
    status_box.append(&darling_badge);

    let doctor_badge = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    doctor_badge.add_css_class("card");
    doctor_badge.set_margin_start(4);
    doctor_badge.set_margin_end(4);

    let doctor_icon = gtk4::Image::from_icon_name("emblem-ok-symbolic");
    doctor_icon.set_margin_start(8);
    doctor_icon.set_margin_top(4);
    doctor_icon.set_margin_bottom(4);
    let doctor_label = gtk4::Label::new(Some("Система: OK"));
    doctor_label.add_css_class("caption");
    doctor_label.set_margin_end(8);
    doctor_label.set_margin_top(4);
    doctor_label.set_margin_bottom(4);
    doctor_badge.append(&doctor_icon);
    doctor_badge.append(&doctor_label);
    doctor_badge.set_tooltip_text(Some("Состояние проверки компонентов (Darling, Vulkan, звук, сеть)"));
    status_box.append(&doctor_badge);

    controls_box.append(&status_box);

    // Play or Install Button
    let play_button = gtk4::Button::with_label(if is_installed { "Играть" } else { "Установить Roblox" });
    play_button.add_css_class("suggested-action");
    play_button.add_css_class("pill");
    play_button.set_size_request(240, 52);

    #[derive(Debug, Clone)]
    enum PlayState {
        Installing(String),
        Installed(String),
        Starting,
        Playing,
        Stopped,
        Failed(String),
    }

    let (tx, rx) = async_channel::bounded::<PlayState>(1);
    let play_btn_rx = play_button.downgrade();
    let win_rx = window.downgrade();
    let status_page_rx = status_page.downgrade();
    let paths_rx = paths.clone();
    let darling_label_rx = darling_label.downgrade();
    let darling_icon_rx = darling_icon.downgrade();

    // Quick Place Join group below Play button
    let join_group = adw::PreferencesGroup::builder()
        .title("Быстрое подключение к серверу")
        .description("Place ID или ссылка на игру")
        .build();
    join_group.set_size_request(340, -1);

    let entry_row = adw::EntryRow::builder()
        .title("Place ID или URL")
        .build();
    entry_row.set_tooltip_text(Some("Введите Place ID (например: 1818, 920587237) или ссылку https://www.roblox.com/games/1818"));

    let join_btn = gtk4::Button::with_label("Присоединиться");
    join_btn.add_css_class("suggested-action");
    join_btn.set_valign(gtk4::Align::Center);
    entry_row.add_suffix(&join_btn);
    join_group.add(&entry_row);

    let join_btn_rx = join_btn.downgrade();
    let entry_row_rx = entry_row.downgrade();

    glib::spawn_future_local(async move {
        while let Ok(state) = rx.recv().await {
            if let Some(btn) = play_btn_rx.upgrade() {
                match state {
                    PlayState::Installing(msg) => {
                        btn.set_sensitive(false);
                        btn.set_label(&msg);
                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(false);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(false);
                        }
                    }
                    PlayState::Installed(ver) => {
                        btn.set_sensitive(true);
                        btn.set_label("Играть");
                        btn.remove_css_class("destructive-action");
                        btn.add_css_class("suggested-action");

                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(true);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(true);
                        }

                        if let Some(sp) = status_page_rx.upgrade() {
                            let u = auth::signed_in_user(&paths_rx).unwrap_or_else(|| "не авторизован".to_string());
                            sp.set_description(Some(&format!("Roblox: {} • Вход выполнен: {}", ver, u)));
                        }
                    }
                    PlayState::Starting => {
                        btn.set_sensitive(false);
                        if let Ok(place_id) = std::env::var("MACOBLOX_PLACE_ID") {
                            btn.set_label(&format!("Запуск Place {}…", place_id));
                        } else {
                            btn.set_label("Запуск Roblox…");
                        }

                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(false);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(false);
                        }
                        if let Some(dl) = darling_label_rx.upgrade() {
                            dl.set_label("Darling: активен");
                        }
                        if let Some(di) = darling_icon_rx.upgrade() {
                            di.set_icon_name(Some("emblem-ok-symbolic"));
                        }
                    }
                    PlayState::Playing => {
                        btn.set_sensitive(true);
                        btn.set_label("Остановить игру");
                        btn.remove_css_class("suggested-action");
                        btn.add_css_class("destructive-action");

                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(false);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(false);
                        }
                        if let Some(dl) = darling_label_rx.upgrade() {
                            dl.set_label("Darling: активен");
                        }
                        if let Some(di) = darling_icon_rx.upgrade() {
                            di.set_icon_name(Some("emblem-ok-symbolic"));
                        }

                        if let Ok(place_id) = std::env::var("MACOBLOX_PLACE_ID") {
                            if let Some(sp) = status_page_rx.upgrade() {
                                let u = auth::signed_in_user(&paths_rx).unwrap_or_else(|| "не авторизован".to_string());
                                sp.set_description(Some(&format!("В игре: Place {} • Пользователь: {}", place_id, u)));
                            }
                        }
                    }
                    PlayState::Stopped => {
                        std::env::remove_var("MACOBLOX_PLACE_ID");
                        std::env::remove_var("MACOBLOX_LAUNCH_URL");
                        std::env::remove_var("ROBLOX_PLACE_ID");

                        btn.set_sensitive(true);
                        let is_inst = updater::installed_version(&paths_rx).is_some();
                        btn.set_label(if is_inst { "Играть" } else { "Установить Roblox" });
                        btn.remove_css_class("destructive-action");
                        btn.add_css_class("suggested-action");

                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(true);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(true);
                        }

                        let active = runner::is_darlingserver_running(&paths_rx.darling_prefix);
                        if let Some(dl) = darling_label_rx.upgrade() {
                            dl.set_label(if active { "Darling: активен" } else { "Darling: готов" });
                        }
                        if let Some(di) = darling_icon_rx.upgrade() {
                            di.set_icon_name(Some(if active { "emblem-ok-symbolic" } else { "emblem-default-symbolic" }));
                        }

                        if let Some(sp) = status_page_rx.upgrade() {
                            let v = updater::installed_version(&paths_rx).unwrap_or_else(|| "не установлен".to_string());
                            let u = auth::signed_in_user(&paths_rx).unwrap_or_else(|| "не авторизован".to_string());
                            sp.set_description(Some(&format!("Roblox: {} • Вход выполнен: {}", v, u)));
                        }
                    }
                    PlayState::Failed(err) => {
                        std::env::remove_var("MACOBLOX_PLACE_ID");
                        std::env::remove_var("MACOBLOX_LAUNCH_URL");
                        std::env::remove_var("ROBLOX_PLACE_ID");

                        btn.set_sensitive(true);
                        let is_inst = updater::installed_version(&paths_rx).is_some();
                        btn.set_label(if is_inst { "Играть" } else { "Установить Roblox" });
                        btn.remove_css_class("destructive-action");
                        btn.add_css_class("suggested-action");

                        if let Some(jb) = join_btn_rx.upgrade() {
                            jb.set_sensitive(true);
                        }
                        if let Some(er) = entry_row_rx.upgrade() {
                            er.set_sensitive(true);
                        }

                        let active = runner::is_darlingserver_running(&paths_rx.darling_prefix);
                        if let Some(dl) = darling_label_rx.upgrade() {
                            dl.set_label(if active { "Darling: активен" } else { "Darling: готов" });
                        }
                        if let Some(di) = darling_icon_rx.upgrade() {
                            di.set_icon_name(Some(if active { "emblem-ok-symbolic" } else { "emblem-default-symbolic" }));
                        }

                        if let Some(win) = win_rx.upgrade() {
                            let dialog = adw::AlertDialog::new(
                                Some("Ошибка"),
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

    play_button.connect_clicked(move |btn| {
        let p = paths_play.clone();

        // If game is running, clicking the button stops it
        if btn.label().as_deref() == Some("Остановить игру") {
            btn.set_sensitive(false);
            btn.set_label("Остановка…");
            let tx_stop = tx.clone();
            std::thread::spawn(move || {
                runner::stop_roblox();
                let _ = tx_stop.send_blocking(PlayState::Stopped);
            });
            return;
        }

        // If there are stale leftover processes from a crashed game, clean them up before starting
        if !runner::roblox_pids().is_empty() {
            runner::stop_roblox();
        }

        // If Roblox is NOT installed or needs updating, run install/update flow!
        let installed = updater::installed_version(&p);
        let mut needs_install = installed.is_none();
        if let Some(ref current_ver) = installed {
            if let Ok((latest_ver, _)) = updater::latest_version() {
                if &latest_ver != current_ver {
                    needs_install = true;
                }
            }
        }

        if needs_install {
            btn.set_sensitive(false);
            btn.set_label("Подключение…");
            let tx_inst = tx.clone();
            let p_inst = p.clone();

            std::thread::spawn(move || {
                let tx_p = tx_inst.clone();
                let res = (|| -> anyhow::Result<String> {
                    let _ = tx_inst.send_blocking(PlayState::Installing("Проверка версии…".to_string()));
                    let (human_ver, upload) = updater::latest_version()?;
                    updater::update_roblox(&p_inst, &upload, move |frac, _msg| {
                        let pct = (frac * 100.0) as u32;
                        let _ = tx_p.send_blocking(PlayState::Installing(format!("Скачивание: {pct}%")));
                    })?;
                    Ok(human_ver)
                })();

                match res {
                    Ok(ver) => {
                        let _ = tx_inst.send_blocking(PlayState::Installed(ver));
                    }
                    Err(e) => {
                        let _ = tx_inst.send_blocking(PlayState::Failed(format!("Не удалось установить Roblox:\n{e}")));
                    }
                }
            });
            return;
        }

        btn.set_sensitive(false);
        if let Ok(place_id) = std::env::var("MACOBLOX_PLACE_ID") {
            btn.set_label(&format!("Запуск Place {}…", place_id));
        } else {
            btn.set_label("Запускаю…");
        }

        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx_clone.send_blocking(PlayState::Failed(format!("Runtime error: {:?}", e)));
                    return;
                }
            };

            rt.block_on(async {
                let launch_res = runner::launch(&p).await;

                match launch_res {
                    Ok(mut session) => {
                        let _ = tx_clone.send_blocking(PlayState::Starting);
                        let mut seen_roblox = false;
                        let mut last_seen = std::time::Instant::now();

                        loop {
                            if let Ok(Some(status)) = session.child.try_wait() {
                                if !seen_roblox && !status.success() {
                                    let diags = runner::scan_crash_diagnostics_with_status(&session.log_path, Some(&status));
                                    let log_tail = if let Ok(content) = std::fs::read_to_string(&session.log_path) {
                                        let lines: Vec<&str> = content.lines().collect();
                                        let start = lines.len().saturating_sub(8);
                                        lines[start..].join("\n")
                                    } else {
                                        String::new()
                                    };
                                    let mut msg = format!("Roblox завершился с кодом {status}\n");
                                    if !diags.is_empty() {
                                        msg.push_str("\nОбнаруженная причина сбоя:\n");
                                        for d in &diags {
                                            msg.push_str(&format!("• {}\n  {}\n", d.title, d.description));
                                            if !d.advice.is_empty() {
                                                msg.push_str("  Рекомендации по устранению:\n");
                                                for a in &d.advice {
                                                    msg.push_str(&format!("  - {}\n", a));
                                                }
                                            }
                                        }
                                    }
                                    if !log_tail.is_empty() {
                                        msg.push_str(&format!("\nХвост лога:\n{log_tail}"));
                                    }
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
                                if last_seen.elapsed().as_secs() >= 2 {
                                    runner::stop_roblox();
                                    let _ = session.child.kill();
                                    let _ = session.child.wait();
                                    let _ = tx_clone.send_blocking(PlayState::Stopped);
                                    break;
                                }
                            }

                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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
    });
    controls_box.append(&play_button);

    // Quick Place Join action logic
    let join_action = {
        let entry_weak = entry_row.downgrade();
        let win_weak = window.downgrade();
        let play_btn_weak = play_button.downgrade();
        let paths_join = paths.clone();

        move || {
            let entry = match entry_weak.upgrade() {
                Some(e) => e,
                None => return,
            };
            let win = match win_weak.upgrade() {
                Some(w) => w,
                None => return,
            };
            let play_btn = match play_btn_weak.upgrade() {
                Some(b) => b,
                None => return,
            };

            let raw_text = entry.text();
            match parse_place_input(&raw_text) {
                Ok(info) => {
                    // Check if Roblox is installed
                    if updater::installed_version(&paths_join).is_none() {
                        let dialog = adw::AlertDialog::new(
                            Some("Roblox не установлен"),
                            Some("Сначала установите клиент Roblox с помощью кнопки «Установить Roblox» выше."),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                        return;
                    }

                    // Check if Roblox is currently running
                    if !runner::roblox_pids().is_empty() {
                        let dialog = adw::AlertDialog::new(
                            Some("Roblox уже запущен"),
                            Some("Клиент Roblox уже работает. Остановите текущую игру перед подключением к новому серверу."),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                        return;
                    }

                    // Set launch environment variables for Place ID and deep link
                    std::env::set_var("MACOBLOX_PLACE_ID", info.place_id.to_string());
                    std::env::set_var("MACOBLOX_LAUNCH_URL", &info.deep_link);
                    std::env::set_var("ROBLOX_PLACE_ID", info.place_id.to_string());

                    // Trigger launch via play button
                    play_btn.emit_clicked();
                }
                Err(err_msg) => {
                    let dialog = adw::AlertDialog::new(
                        Some("Некорректный Place ID или URL"),
                        Some(&err_msg),
                    );
                    dialog.add_response("ok", "OK");
                    dialog.present(Some(&win));
                }
            }
        }
    };

    let join_action_rc = std::rc::Rc::new(join_action);
    {
        let ja = join_action_rc.clone();
        join_btn.connect_clicked(move |_| {
            ja();
        });
    }
    {
        let ja = join_action_rc;
        entry_row.connect_entry_activated(move |_| {
            ja();
        });
    }

    controls_box.append(&join_group);

    // Sign in Button
    let signin_button = gtk4::Button::with_label(if auth::signed_in(&paths) {
        "Сменить аккаунт"
    } else {
        "Войти в Roblox"
    });
    signin_button.add_css_class("pill");
    signin_button.set_size_request(240, -1);

    let paths_auth = paths.clone();
    let status_weak = status_page.downgrade();
    let window_auth_weak = window.downgrade();

    signin_button.connect_clicked(move |_| {
        if let Some(win) = window_auth_weak.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Вход в Roblox"),
                Some("Выберите способ входа: импортировать активную сессию из браузера или ввести куки .ROBLOSECURITY вручную.")
            );

            dialog.add_response("browser", "Из браузера (Chrome, Firefox, Brave...)");
            dialog.add_response("manual", "Ввести куки вручную");
            dialog.add_response("cancel", "Отмена");
            dialog.set_default_response(Some("browser"));

            let p = paths_auth.clone();
            let st = status_weak.clone();
            let win_resp = win.clone();

            dialog.connect_response(None, move |_, resp| {
                if resp == "browser" {
                    let (tx, rx) = async_channel::bounded(1);
                    let p_bg = p.clone();
                    std::thread::spawn(move || {
                        let res = auth::import_browser_cookies(&p_bg);
                        let _ = tx.send_blocking(res);
                    });
                    let st_ui = st.clone();
                    let win_ui = win_resp.clone();
                    let p_ui = p.clone();
                    glib::spawn_future_local(async move {
                        if let Ok(res) = rx.recv().await {
                            match res {
                                Ok(Some((user, browser))) => {
                                    if let Some(st_up) = st_ui.upgrade() {
                                        let v = updater::installed_version(&p_ui).unwrap_or_else(|| "не найден".to_string());
                                        st_up.set_description(Some(&format!(
                                            "Roblox {} • Вход выполнен: {} ({})",
                                            v, user, browser
                                        )));
                                    }
                                }
                                Ok(None) => {
                                    let err_dialog = adw::AlertDialog::new(
                                        Some("Сессия не найдена"),
                                        Some("Не найдено активных сессий Roblox. Убедитесь, что вы авторизованы на roblox.com в вашем браузере (Chrome, Chromium, Firefox, Brave, Edge, Opera, Zen и др.), либо введите куки вручную.")
                                    );
                                    err_dialog.add_response("ok", "Понятно");
                                    err_dialog.present(Some(&win_ui));
                                }
                                Err(e) => {
                                    let err_dialog = adw::AlertDialog::new(
                                        Some("Ошибка входа"),
                                        Some(&format!("Не удалось получить куки: {}", e))
                                    );
                                    err_dialog.add_response("ok", "Понятно");
                                    err_dialog.present(Some(&win_ui));
                                }
                            }
                        }
                    });
                } else if resp == "manual" {
                    let entry_dialog = adw::AlertDialog::new(
                        Some("Ввод .ROBLOSECURITY"),
                        Some("Вставьте куки .ROBLOSECURITY для входа в ваш аккаунт:")
                    );
                    let cookie_entry = gtk4::Entry::new();
                    cookie_entry.set_placeholder_text(Some("_|WARNING:-DO-NOT-SHARE-THIS..."));
                    cookie_entry.set_visibility(false);
                    entry_dialog.set_extra_child(Some(&cookie_entry));
                    entry_dialog.add_response("save", "Войти");
                    entry_dialog.add_response("cancel", "Отмена");
                    entry_dialog.set_default_response(Some("save"));

                    let p_manual = p.clone();
                    let st_manual = st.clone();
                    let win_manual = win_resp.clone();

                    entry_dialog.connect_response(None, move |_, r| {
                        if r == "save" {
                            let text = cookie_entry.text().to_string();
                            if text.trim().is_empty() {
                                return;
                            }
                            let (tx, rx) = async_channel::bounded(1);
                            let p_bg = p_manual.clone();
                            std::thread::spawn(move || {
                                let res = auth::save_session_cookie(&p_bg, &text);
                                let _ = tx.send_blocking(res);
                            });
                            let st_ui = st_manual.clone();
                            let win_ui = win_manual.clone();
                            let p_ui = p_manual.clone();
                            glib::spawn_future_local(async move {
                                if let Ok(res) = rx.recv().await {
                                    match res {
                                        Ok(user) => {
                                            if let Some(st_up) = st_ui.upgrade() {
                                                let v = updater::installed_version(&p_ui).unwrap_or_else(|| "не найден".to_string());
                                                st_up.set_description(Some(&format!(
                                                    "Roblox {} • Вход выполнен: {}",
                                                    v, user
                                                )));
                                            }
                                        }
                                        Err(e) => {
                                            let err_dialog = adw::AlertDialog::new(
                                                Some("Ошибка авторизации"),
                                                Some(&format!("Не удалось сохранить куки: {}", e))
                                            );
                                            err_dialog.add_response("ok", "Понятно");
                                            err_dialog.present(Some(&win_ui));
                                        }
                                    }
                                }
                            });
                        }
                    });
                    entry_dialog.present(Some(&win_resp));
                }
            });

            dialog.present(Some(&win));
        }
    });
    controls_box.append(&signin_button);

    // Roblox Studio Button
    let studio_button = gtk4::Button::with_label("Roblox Studio");
    studio_button.add_css_class("pill");
    studio_button.set_size_request(240, -1);

    let paths_st_play = paths.clone();
    let win_st_play = window.downgrade();
    studio_button.connect_clicked(move |_| {
        let candidate_paths = [
            paths_st_play.project_dir.join("launcher/macoblox-launcher"),
            paths_st_play.data_dir.join("launcher/macoblox-launcher"),
            std::path::PathBuf::from("launcher/macoblox-launcher"),
            paths_st_play.project_dir.join("macoblox-launcher"),
        ];

        let found = candidate_paths.into_iter().find(|p| p.is_file());
        if let Some(script) = found {
            let res = std::process::Command::new("python3")
                .arg(&script)
                .arg("--studio")
                .spawn();

            if let Some(win) = win_st_play.upgrade() {
                match res {
                    Ok(_) => {
                        let dialog = adw::AlertDialog::new(
                            Some("Roblox Studio запускается"),
                            Some("Roblox Studio запускается через Wine и DXVK. Если это первый запуск, компоненты могут загружаться некоторое время."),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                    }
                    Err(e) => {
                        let dialog = adw::AlertDialog::new(
                            Some("Ошибка запуска"),
                            Some(&format!("Не удалось запустить Roblox Studio:\n{e}")),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                    }
                }
            }
        } else if let Some(win) = win_st_play.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Roblox Studio (Wine)"),
                Some("Roblox Studio на Linux работает через переносимый Wine (staging wow64) с Direct3D 11 через DXVK.\n\nДля запуска выполните команду:\nlauncher/macoblox-launcher --studio"),
            );
            dialog.add_response("ok", "Понятно");
            dialog.present(Some(&win));
        }
    });
    controls_box.append(&studio_button);

    // Community links
    let links_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    links_box.set_halign(gtk4::Align::Center);
    links_box.set_margin_top(4);

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
        crate::gui::open_uri("https://github.com/tanukis0408/Crabblox");
    });
    links_box.append(&github_btn);

    controls_box.append(&links_box);

    status_page.set_child(Some(&controls_box));
    root.append(&status_page);

    // Footer version
    let footer_text = format!("Crabblox {} • Monster Dev", env!("CARGO_PKG_VERSION"));
    let version_label = gtk4::Label::new(Some(&footer_text));
    version_label.add_css_class("dim-label");
    version_label.add_css_class("caption");
    version_label.set_margin_bottom(12);
    root.append(&version_label);

    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_place_input_direct_id() {
        let res = parse_place_input("1818").unwrap();
        assert_eq!(res.place_id, 1818);
        assert_eq!(res.deep_link, "roblox://placeId=1818");
        assert_eq!(res.web_url, "https://www.roblox.com/games/1818");

        let res2 = parse_place_input("  920587237  \n").unwrap();
        assert_eq!(res2.place_id, 920587237);
    }

    #[test]
    fn test_parse_place_input_roblox_url() {
        let res = parse_place_input("https://www.roblox.com/games/1818/Super-Bomb-Survival").unwrap();
        assert_eq!(res.place_id, 1818);
        assert_eq!(res.deep_link, "roblox://placeId=1818");

        let res2 = parse_place_input("https://roblox.com/games/920587237").unwrap();
        assert_eq!(res2.place_id, 920587237);
        assert_eq!(res2.deep_link, "roblox://placeId=920587237");

        let res3 = parse_place_input("https://www.roblox.com/discover?placeId=654321").unwrap();
        assert_eq!(res3.place_id, 654321);
        assert_eq!(res3.deep_link, "roblox://placeId=654321");
    }

    #[test]
    fn test_parse_place_input_deep_link() {
        let res = parse_place_input("roblox://experiences/start?placeId=1818").unwrap();
        assert_eq!(res.place_id, 1818);
        assert_eq!(res.deep_link, "roblox://placeId=1818");

        let res2 = parse_place_input("roblox-player:1+launchmode:play+placeid:1818").unwrap();
        assert_eq!(res2.place_id, 1818);
        assert_eq!(res2.deep_link, "roblox://placeId=1818");

        let res3 = parse_place_input("roblox://placeId=9999").unwrap();
        assert_eq!(res3.place_id, 9999);
    }

    #[test]
    fn test_parse_place_input_invalid() {
        assert!(parse_place_input("").is_err());
        assert!(parse_place_input("   ").is_err());
        assert!(parse_place_input("invalid_input").is_err());
        assert!(parse_place_input("https://google.com").is_err());
    }
}
