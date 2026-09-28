use std::sync::Arc;
use adw::prelude::*;
use gtk4::prelude::*;
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
                        let _ = std::fs::remove_file(p.cookies_plist());
                        let _ = std::fs::remove_file(p.user_cache_file());
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
                        Some("Войдите на roblox.com в браузере и повторите попытку.")
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
    roblox_group.add(&version_row);

    page.add(&roblox_group);

    // 4. Diagnostics Group
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

    page.add(&diag_group);

    page
}
