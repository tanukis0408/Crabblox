use std::sync::Arc;
use adw::prelude::*;
use crate::auth;
use crate::paths::Paths;
use crate::updater;

pub fn build_settings_page(window: &adw::ApplicationWindow, paths: Arc<Paths>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // 1. Current Account Group
    let account_group = adw::PreferencesGroup::builder()
        .title("Текущая сессия")
        .build();

    let user_name = auth::signed_in_user(&paths);
    let signed_in = user_name.is_some();

    let account_row = adw::ActionRow::builder()
        .title(if let Some(ref u) = user_name {
            format!("Активен: {}", u)
        } else {
            "Вход не выполнен".to_string()
        })
        .subtitle("Сессия сохранена в Cookies.plist")
        .build();

    if signed_in {
        let signout_btn = gtk4::Button::with_label("Выйти");
        signout_btn.add_css_class("destructive-action");
        signout_btn.set_valign(gtk4::Align::Center);

        let paths_so = paths.clone();
        let row_weak = account_row.downgrade();
        let win_weak = window.downgrade();

        signout_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                let dialog = adw::AlertDialog::new(
                    Some("Выйти из аккаунта?"),
                    Some("Сохранённая сессия Roblox будет удалена, при следующем запуске потребуется войти заново.")
                );
                dialog.add_response("cancel", "Отмена");
                dialog.add_response("signout", "Выйти");
                dialog.set_response_appearance("signout", adw::ResponseAppearance::Destructive);

                let p = paths_so.clone();
                let r = row_weak.clone();

                dialog.connect_response(None, move |_, resp| {
                    if resp == "signout" {
                        auth::sign_out(&p);
                        if let Some(r_up) = r.upgrade() {
                            r_up.set_title("Вход не выполнен");
                            r_up.set_subtitle("Авторизуйтесь на вкладке «Играть»");
                        }
                    }
                });

                dialog.present(Some(&win));
            }
        });
        account_row.add_suffix(&signout_btn);
    }

    account_group.add(&account_row);
    page.add(&account_group);

    // 2. Multi-Account Manager Group
    let accounts_group = adw::PreferencesGroup::builder()
        .title("Менеджер аккаунтов (Account Switcher)")
        .description("Переключение между сохранёнными профилями в 1 клик")
        .build();

    let saved_accounts = auth::load_accounts(&paths);
    let cur_user = auth::signed_in_user(&paths);

    for acc in &saved_accounts {
        let is_current = cur_user.as_deref() == Some(&acc.display_name) || cur_user.as_deref() == Some(&acc.username);
        let sub = if is_current { "Активен".to_string() } else { format!("@{}", acc.username) };
        let row = adw::ActionRow::builder()
            .title(&acc.display_name)
            .subtitle(&sub)
            .build();

        if is_current {
            let active_badge = gtk4::Image::from_icon_name("emblem-ok-symbolic");
            active_badge.set_valign(gtk4::Align::Center);
            row.add_suffix(&active_badge);
        } else {
            let switch_btn = gtk4::Button::with_label("Выбрать");
            switch_btn.add_css_class("suggested-action");
            switch_btn.set_valign(gtk4::Align::Center);
            let uid = acc.id;
            let paths_sw = paths.clone();
            let win_sw = window.downgrade();
            let cur_row_weak = account_row.downgrade();

            switch_btn.connect_clicked(move |_| {
                if let Ok(user) = auth::switch_to_account(&paths_sw, uid) {
                    if let Some(cr) = cur_row_weak.upgrade() {
                        cr.set_title(&format!("Активен: {}", user));
                    }
                    if let Some(w) = win_sw.upgrade() {
                        let ok_diag = adw::AlertDialog::new(
                            Some("Аккаунт переключен"),
                            Some(&format!("Текущий активный аккаунт: {}", user))
                        );
                        ok_diag.add_response("ok", "Отлично");
                        ok_diag.present(Some(&w));
                    }
                }
            });
            row.add_suffix(&switch_btn);
        }

        // Delete button
        let del_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
        del_btn.add_css_class("flat");
        del_btn.set_valign(gtk4::Align::Center);
        let uid = acc.id;
        let paths_del = paths.clone();
        let row_weak = row.downgrade();
        del_btn.connect_clicked(move |_| {
            let _ = auth::remove_saved_account(&paths_del, uid);
            if let Some(r) = row_weak.upgrade() {
                r.set_visible(false);
            }
        });
        row.add_suffix(&del_btn);

        accounts_group.add(&row);
    }

    // Add another account action
    let add_acc_row = adw::ActionRow::builder()
        .title("Добавить новый аккаунт из браузера")
        .subtitle("Импортирует сессию из Chrome, Firefox или Brave")
        .activatable(true)
        .build();
    add_acc_row.add_suffix(&gtk4::Image::from_icon_name("list-add-symbolic"));

    let paths_add = paths.clone();
    let win_add = window.downgrade();
    add_acc_row.connect_activated(move |_| {
        if let Some(w) = win_add.upgrade() {
            match auth::import_browser_cookies(&paths_add) {
                Ok(Some((user, bname))) => {
                    let d = adw::AlertDialog::new(
                        Some("Аккаунт добавлен"),
                        Some(&format!("Успешно импортирован аккаунт '{}' из {}. Перезайдите в настройки, чтобы увидеть его в списке.", user, bname))
                    );
                    d.add_response("ok", "Готово");
                    d.present(Some(&w));
                }
                Ok(None) => {
                    let d = adw::AlertDialog::new(
                        Some("Сессия не найдена"),
                        Some("Не найдено активных сессий Roblox. Убедитесь, что вы авторизованы на roblox.com в браузере (Chrome, Chromium, Firefox, Brave, Edge, Opera, Zen и др.).")
                    );
                    d.add_response("ok", "Понятно");
                    d.present(Some(&w));
                }
                Err(e) => {
                    let d = adw::AlertDialog::new(
                        Some("Ошибка"),
                        Some(&format!("Не удалось получить куки: {}", e))
                    );
                    d.add_response("ok", "Понятно");
                    d.present(Some(&w));
                }
            }
        }
    });
    accounts_group.add(&add_acc_row);

    page.add(&accounts_group);

    // 3. Roblox Version Group
    let roblox_group = adw::PreferencesGroup::builder()
        .title("Клиент Roblox")
        .build();

    let version_str = updater::installed_version(&paths).unwrap_or_else(|| "Не установлен".to_string());
    let version_row = adw::ActionRow::builder()
        .title("Установленная версия")
        .subtitle(&version_str)
        .build();

    let update_btn = gtk4::Button::with_label("Обновить");
    update_btn.add_css_class("suggested-action");
    update_btn.set_valign(gtk4::Align::Center);

    let paths_up = paths.clone();
    let win_up = window.downgrade();
    let vrow_weak = version_row.downgrade();
    let btn_weak = update_btn.downgrade();

    update_btn.connect_clicked(move |_| {
        let p = paths_up.clone();
        let w_opt = win_up.upgrade();
        let vr_opt = vrow_weak.upgrade();
        let b_opt = btn_weak.upgrade();

        if let Some(ref b) = b_opt {
            b.set_sensitive(false);
            b.set_label("Проверка…");
        }

        let (tx, rx) = async_channel::bounded::<Result<(String, bool), String>>(1);

        std::thread::spawn(move || {
            let res = (|| -> anyhow::Result<(String, bool)> {
                let (latest_ver, upload) = updater::latest_version()?;
                let installed = updater::installed_version(&p);
                if installed.as_deref() != Some(&latest_ver) {
                    updater::update_roblox(&p, &upload, |_, _| {})?;
                    Ok((latest_ver, true))
                } else {
                    Ok((latest_ver, false))
                }
            })();

            let _ = tx.send_blocking(res.map_err(|e| e.to_string()));
        });

        glib::spawn_future_local(async move {
            if let Ok(res) = rx.recv().await {
                if let Some(b) = b_opt {
                    b.set_sensitive(true);
                    b.set_label("Обновить");
                }
                match res {
                    Ok((ver, updated)) => {
                        if let Some(vr) = vr_opt {
                            vr.set_subtitle(&ver);
                        }
                        if let Some(w) = w_opt {
                            let msg = if updated {
                                format!("Roblox успешно обновлен до версии {ver}")
                            } else {
                                format!("У вас уже установлена актуальная версия {ver}")
                            };
                            let d = adw::AlertDialog::new(Some("Обновление Roblox"), Some(&msg));
                            d.add_response("ok", "OK");
                            d.present(Some(&w));
                        }
                    }
                    Err(e) => {
                        if let Some(w) = w_opt {
                            let d = adw::AlertDialog::new(Some("Ошибка обновления"), Some(&e));
                            d.add_response("ok", "OK");
                            d.present(Some(&w));
                        }
                    }
                }
            }
        });
    });

    version_row.add_suffix(&update_btn);
    roblox_group.add(&version_row);

    page.add(&roblox_group);

    // 4. Roblox Studio (Wine) Group
    let studio_group = adw::PreferencesGroup::builder()
        .title("Roblox Studio (Wine)")
        .description("Среда разработки Roblox Studio работает через Wine (staging wow64) и DXVK")
        .build();

    let studio_row = adw::ActionRow::builder()
        .title("Запустить Roblox Studio")
        .subtitle("Запуск Windows-версии Studio через Wine с поддержкой Direct3D 11")
        .build();

    let launch_studio_btn = gtk4::Button::with_label("Запустить Roblox Studio");
    launch_studio_btn.add_css_class("suggested-action");
    launch_studio_btn.set_valign(gtk4::Align::Center);

    let paths_studio = paths.clone();
    let win_studio = window.downgrade();
    launch_studio_btn.connect_clicked(move |_| {
        let candidate_paths = [
            paths_studio.project_dir.join("launcher/macoblox-launcher"),
            paths_studio.data_dir.join("launcher/macoblox-launcher"),
            std::path::PathBuf::from("launcher/macoblox-launcher"),
            paths_studio.project_dir.join("macoblox-launcher"),
        ];

        let found_script = candidate_paths.into_iter().find(|p| p.is_file());

        if let Some(script) = found_script {
            let res = std::process::Command::new("python3")
                .arg(&script)
                .arg("--studio")
                .spawn();

            if let Some(win) = win_studio.upgrade() {
                match res {
                    Ok(_) => {
                        let dialog = adw::AlertDialog::new(
                            Some("Roblox Studio запускается"),
                            Some("Команда запуска Roblox Studio через Wine отправлена. Если это первый запуск, Wine и компоненты Studio могут загружаться некоторое время."),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                    }
                    Err(e) => {
                        let dialog = adw::AlertDialog::new(
                            Some("Ошибка запуска"),
                            Some(&format!("Не удалось выполнить запуск:\n{e}")),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                    }
                }
            }
        } else if let Some(win) = win_studio.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Roblox Studio (Wine)"),
                Some("Скрипт `launcher/macoblox-launcher` не найден.\n\nRoblox Studio на Linux работает через портативный Wine (Kron4ek staging wow64) с трансляцией Direct3D 11 в Vulkan через DXVK. Все компоненты и префикс загружаются автоматически в каталог данных Crabblox (директория studio/).\n\nДля запуска используйте команду в терминале:\nlauncher/macoblox-launcher --studio"),
            );
            dialog.add_response("ok", "Понятно");
            dialog.present(Some(&win));
        }
    });

    studio_row.add_suffix(&launch_studio_btn);
    studio_group.add(&studio_row);
    page.add(&studio_group);

    // 5. Diagnostics Group
    let diag_group = adw::PreferencesGroup::builder()
        .title("Диагностика")
        .description("Инструменты отладки и просмотра логов")
        .build();

    let open_logs_row = adw::ActionRow::builder()
        .title("Открыть папку с логами")
        .subtitle(&*paths.logs_dir().to_string_lossy())
        .activatable(true)
        .build();

    let logs_dir = paths.logs_dir();
    open_logs_row.connect_activated(move |_| {
        let uri = format!("file://{}", logs_dir.display());
        crate::gui::open_uri(&uri);
    });
    diag_group.add(&open_logs_row);

    // Screenshots and recordings row
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let host_pictures = dirs::picture_dir().unwrap_or_else(|| home.join("Pictures")).join("Roblox");
    let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let darling_pictures = paths.darling_prefix
        .join("Users")
        .join(&username)
        .join("Pictures/Roblox");

    let display_pics_path = if darling_pictures.exists() {
        darling_pictures.display().to_string()
    } else {
        host_pictures.display().to_string()
    };

    let screenshots_row = adw::ActionRow::builder()
        .title("Скриншоты и видеозаписи")
        .subtitle(&display_pics_path)
        .activatable(true)
        .build();
    screenshots_row.add_suffix(&gtk4::Image::from_icon_name("folder-pictures-symbolic"));

    let paths_scr = paths.clone();
    screenshots_row.connect_activated(move |_| {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
        let host_pics = dirs::picture_dir().unwrap_or_else(|| home.join("Pictures")).join("Roblox");
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        let darling_pics = paths_scr.darling_prefix
            .join("Users")
            .join(&username)
            .join("Pictures/Roblox");

        let target_dir = if darling_pics.exists() {
            darling_pics
        } else if host_pics.exists() {
            host_pics
        } else {
            let _ = std::fs::create_dir_all(&host_pics);
            host_pics
        };

        let uri = format!("file://{}", target_dir.display());
        crate::gui::open_uri(&uri);
    });
    diag_group.add(&screenshots_row);

    let restart_darling_row = adw::ActionRow::builder()
        .title("Перезапустить Darling")
        .subtitle("Останавливает darlingserver, запустится при следующей игре")
        .activatable(true)
        .build();

    let paths_restart = paths.clone();
    let win_restart = window.downgrade();
    restart_darling_row.connect_activated(move |_| {
        crate::runner::restart_darling(&paths_restart.darling_prefix);
        if let Some(win) = win_restart.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Darling перезапущен"),
                Some("Службы контейнера Darling и darlingserver успешно остановлены и очищены. Они автоматически перезапустятся при следующем входе в игру."),
            );
            dialog.add_response("ok", "OK");
            dialog.present(Some(&win));
        }
    });
    diag_group.add(&restart_darling_row);

    // System Doctor row
    let doctor_row = adw::ActionRow::builder()
        .title("Системный доктор (Health Check)")
        .subtitle("Комплексная диагностика Darling, Vulkan, звука, лимитов системы и сети")
        .build();

    let doctor_btn = gtk4::Button::with_label("Проверить");
    doctor_btn.add_css_class("suggested-action");
    doctor_btn.set_valign(gtk4::Align::Center);

    let paths_doc = paths.clone();
    let win_doc = window.downgrade();
    let doc_btn_weak = doctor_btn.downgrade();

    doctor_btn.connect_clicked(move |_| {
        let p = paths_doc.clone();
        let w_opt = win_doc.upgrade();
        let b_opt = doc_btn_weak.upgrade();

        if let Some(ref b) = b_opt {
            b.set_sensitive(false);
            b.set_label("Проверка…");
        }

        let (tx, rx) = async_channel::bounded::<crate::doctor::DoctorReport>(1);

        std::thread::spawn(move || {
            let report = crate::doctor::DoctorReport::run(&p);
            let _ = tx.send_blocking(report);
        });

        glib::spawn_future_local(async move {
            if let Ok(report) = rx.recv().await {
                if let Some(b) = b_opt {
                    b.set_sensitive(true);
                    b.set_label("Проверить");
                }
                if let Some(w) = w_opt {
                    let (_, w_cnt, f_cnt) = report.summary_counts();
                    let heading = if f_cnt > 0 {
                        "Системный доктор: обнаружены проблемы"
                    } else if w_cnt > 0 {
                        "Системный доктор: предупреждения"
                    } else {
                        "Системный доктор: всё готово к запуску"
                    };

                    let details = report.to_dialog_text();
                    let dialog = adw::AlertDialog::new(Some(heading), Some(&details));
                    dialog.add_response("copy", "Копировать отчёт");
                    dialog.add_response("ok", "Закрыть");
                    dialog.set_default_response(Some("ok"));

                    let text_to_copy = details.clone();
                    dialog.connect_response(None, move |_, resp| {
                        if resp == "copy" {
                            if let Some(display) = gtk4::gdk::Display::default() {
                                display.clipboard().set_text(&text_to_copy);
                            }
                        }
                    });

                    dialog.present(Some(&w));
                }
            }
        });
    });

    doctor_row.add_suffix(&doctor_btn);
    diag_group.add(&doctor_row);

    page.add(&diag_group);

    page
}
