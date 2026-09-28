use std::sync::Arc;
use adw::prelude::*;
use crate::fast_flags::{FastFlags, LightingTechnology};
use crate::paths::Paths;

pub fn build_flags_page(window: &adw::ApplicationWindow, paths: Arc<Paths>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // 1. Presets Group
    let presets_group = adw::PreferencesGroup::builder()
        .title("Игровые пресеты")
        .description("Быстрая настройка производительности и приватности")
        .build();

    // Potato mode switch
    let potato_row = adw::SwitchRow::builder()
        .title("Режим «Картошка» (Potato Mode / Макс. FPS)")
        .subtitle("Отключает пост-эффекты, тени, траву, облака и снижает нагрузку на слабых ПК")
        .build();
    potato_row.set_active(FastFlags::is_potato_mode(&paths));

    // Ultra graphics switch
    let ultra_row = adw::SwitchRow::builder()
        .title("Ультра-графика (Ultra Graphics)")
        .subtitle("Максимальное качество рендеринга (Level 21), 8x сглаживание MSAA и дальность прорисовки")
        .build();
    ultra_row.set_active(FastFlags::is_ultra_mode(&paths));

    let paths_pot = paths.clone();
    let ultra_weak = ultra_row.downgrade();
    potato_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_potato_mode(&paths_pot, active);
        if active {
            if let Some(u) = ultra_weak.upgrade() {
                u.set_active(false);
            }
        }
    });
    presets_group.add(&potato_row);

    let paths_ult = paths.clone();
    let pot_weak = potato_row.downgrade();
    ultra_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_ultra_mode(&paths_ult, active);
        if active {
            if let Some(p) = pot_weak.upgrade() {
                p.set_active(false);
            }
        }
    });
    presets_group.add(&ultra_row);

    // Telemetry switch
    let telemetry_row = adw::SwitchRow::builder()
        .title("Отключить телеметрию и сбор данных")
        .subtitle("Блокирует отправку трекинга и аналитики в Roblox (снижает сетевой лаг)")
        .build();
    telemetry_row.set_active(FastFlags::is_telemetry_disabled(&paths));

    let paths_tel = paths.clone();
    telemetry_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_disable_telemetry(&paths_tel, active);
    });
    presets_group.add(&telemetry_row);

    page.add(&presets_group);

    // 2. Освещение и графика (Future is Bright Phase 3, Classic Materials, Max Texture Detail)
    let lighting_group = adw::PreferencesGroup::builder()
        .title("Освещение и графика")
        .description("Настройки движка рендеринга, технологии света и материалов")
        .build();

    // Lighting technology combo row
    let lighting_model = gtk4::StringList::new(&[
        "По умолчанию (выбор игры)",
        "Voxel (Фаза 1 — классическое освещение)",
        "ShadowMap (Фаза 2 — мягкие тени)",
        "Future is Bright (Фаза 3 — реалистичный свет)",
    ]);
    let lighting_row = adw::ComboRow::builder()
        .title("Технология освещения (Future is Bright Phase 3)")
        .subtitle("Future is Bright Phase 3 / ShadowMap / Voxel (DFFlagDebugRenderForceTechnology)")
        .model(&lighting_model)
        .build();
    let cur_tech = FastFlags::get_lighting_technology(&paths);
    lighting_row.set_selected(cur_tech.as_u32());

    let paths_lt = paths.clone();
    lighting_row.connect_selected_notify(move |row| {
        let tech = LightingTechnology::from_u32(row.selected());
        let _ = FastFlags::apply_lighting_technology(&paths_lt, tech);
    });
    lighting_group.add(&lighting_row);

    // Classic 2021 pre-update materials switch
    let classic_mat_row = adw::SwitchRow::builder()
        .title("Классические материалы 2021 (Classic Materials)")
        .subtitle("FFlagFixGraphicsQuality, DFFlagDisableNewMaterials2022 — возврат текстур до обновления 2022 года")
        .build();
    classic_mat_row.set_active(FastFlags::is_classic_materials(&paths));

    let paths_cm = paths.clone();
    classic_mat_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_classic_materials(&paths_cm, active);
    });
    lighting_group.add(&classic_mat_row);

    // Max Texture Detail switch
    let tex_detail_row = adw::SwitchRow::builder()
        .title("Максимальная детализация текстур (Max Texture Detail)")
        .subtitle("FIntRenderTextureDetail, DFIntTextureQualityOverride — отключение деградации LOD и замыливания текстур")
        .build();
    tex_detail_row.set_active(FastFlags::is_max_texture_detail(&paths));

    let paths_td = paths.clone();
    tex_detail_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_max_texture_detail(&paths_td, active);
    });
    lighting_group.add(&tex_detail_row);

    page.add(&lighting_group);

    // 3. Интерфейс (Chrome UI)
    let interface_group = adw::PreferencesGroup::builder()
        .title("Интерфейс")
        .description("Настройки внутриигрового пользовательского меню Roblox")
        .build();

    let chrome_row = adw::SwitchRow::builder()
        .title("Новый интерфейс меню (Chrome UI)")
        .subtitle("FFlagEnableInGameMenuChrome — принудительное включение обновлённого верхнего меню в игре")
        .build();
    chrome_row.set_active(FastFlags::is_chrome_ui_enabled(&paths));

    let paths_chr = paths.clone();
    chrome_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let _ = FastFlags::apply_chrome_ui(&paths_chr, active);
    });
    interface_group.add(&chrome_row);

    page.add(&interface_group);

    // 4. Точечные настройки
    let popular_group = adw::PreferencesGroup::builder()
        .title("Точечные настройки")
        .description("Индивидуальные параметры движка")
        .build();

    // FPS Cap Row
    let fps_row = adw::ActionRow::builder()
        .title("Лимит кадров (FPS)")
        .subtitle("DFIntTaskSchedulerTargetFps (0 = по умолчанию)")
        .build();

    let fps_spin = gtk4::SpinButton::with_range(0.0, 999.0, 30.0);
    let current_flags = FastFlags::load(&paths);
    let cur_fps = current_flags.get("DFIntTaskSchedulerTargetFps")
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as f64;
    fps_spin.set_value(cur_fps);

    let paths_fps = paths.clone();
    fps_spin.connect_value_changed(move |spin| {
        let val = spin.value() as u32;
        let _ = FastFlags::set_fps_cap(&paths_fps, val);
    });
    fps_row.add_suffix(&fps_spin);
    popular_group.add(&fps_row);

    // No shadows switch row
    let shadows_row = adw::SwitchRow::builder()
        .title("Отключить тени")
        .subtitle("FIntRenderShadowIntensity = 0")
        .build();
    let shadows_active = current_flags.get("FIntRenderShadowIntensity")
        .and_then(|v| v.as_str().or(v.as_i64().map(|_| "")))
        .map(|v| v == "0" || v == "")
        .unwrap_or(false);
    shadows_row.set_active(shadows_active);

    let paths_sh = paths.clone();
    shadows_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let val = if active { serde_json::json!("0") } else { serde_json::json!("1") };
        let _ = FastFlags::set_flag(&paths_sh, "FIntRenderShadowIntensity", val);
    });
    popular_group.add(&shadows_row);

    // No grass switch row
    let grass_row = adw::SwitchRow::builder()
        .title("Отключить траву")
        .subtitle("FIntFRMMinGrassDistance = 0")
        .build();
    let grass_active = current_flags.get("FIntFRMMinGrassDistance")
        .and_then(|v| v.as_str().or(v.as_i64().map(|_| "")))
        .map(|v| v == "0" || v == "")
        .unwrap_or(false);
    grass_row.set_active(grass_active);

    let paths_gr = paths.clone();
    grass_row.connect_active_notify(move |row| {
        let active = row.is_active();
        let val = if active { serde_json::json!("0") } else { serde_json::json!("100") };
        let _ = FastFlags::set_flag(&paths_gr, "FIntFRMMinGrassDistance", val);
    });
    popular_group.add(&grass_row);

    page.add(&popular_group);

    // 5. Импорт / Экспорт JSON
    let json_group = adw::PreferencesGroup::builder()
        .title("Импорт / Экспорт JSON")
        .description("Управление и обмен произвольными FastFlags в формате JSON")
        .build();

    // Import Row with Button
    let import_row = adw::ActionRow::builder()
        .title("Импорт FastFlags")
        .subtitle("Вставить и объединить флаги из JSON")
        .build();

    let import_btn = gtk4::Button::with_label("Вставить FastFlags из буфера / JSON");
    import_btn.add_css_class("suggested-action");
    import_btn.set_valign(gtk4::Align::Center);

    let win_weak_imp = window.downgrade();
    let paths_imp = paths.clone();
    import_btn.connect_clicked(move |_| {
        if let Some(win) = win_weak_imp.upgrade() {
            let dialog = adw::AlertDialog::new(
                Some("Импорт FastFlags из JSON"),
                Some("Вставьте JSON-объект с флагами ниже для добавления или обновления настроек:")
            );
            dialog.add_response("cancel", "Отмена");
            dialog.add_response("import", "Импортировать");
            dialog.set_response_appearance("import", adw::ResponseAppearance::Suggested);

            let text_view = gtk4::TextView::new();
            text_view.set_monospace(true);
            text_view.set_wrap_mode(gtk4::WrapMode::Char);
            text_view.set_top_margin(8);
            text_view.set_bottom_margin(8);
            text_view.set_left_margin(8);
            text_view.set_right_margin(8);

            let buffer = text_view.buffer();
            buffer.set_text("{\n  \n}");

            // Try to auto-populate from clipboard if it contains JSON
            if let Some(display) = gtk4::gdk::Display::default() {
                let clipboard = display.clipboard();
                let b = buffer.clone();
                clipboard.read_text_async(gio::Cancellable::NONE, move |res| {
                    if let Ok(Some(text)) = res {
                        let trimmed = text.trim();
                        if trimmed.starts_with('{') && trimmed.ends_with('}') {
                            b.set_text(trimmed);
                        }
                    }
                });
            }

            let scrolled = gtk4::ScrolledWindow::builder()
                .child(&text_view)
                .min_content_height(180)
                .max_content_height(320)
                .min_content_width(400)
                .has_frame(true)
                .build();

            let content_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
            content_box.append(&scrolled);

            dialog.set_extra_child(Some(&content_box));

            let p = paths_imp.clone();
            let w_weak = win.downgrade();
            dialog.connect_response(None, move |_, resp| {
                if resp == "import" {
                    let start = buffer.start_iter();
                    let end = buffer.end_iter();
                    let text = buffer.text(&start, &end, false).to_string();

                    if let Some(parent_win) = w_weak.upgrade() {
                        match FastFlags::import_json(&p, &text) {
                            Ok(count) => {
                                let ok_dialog = adw::AlertDialog::new(
                                    Some("Импорт завершён"),
                                    Some(&format!("Успешно импортировано флагов: {count}. Настройки сохранены в ClientAppSettings.json."))
                                );
                                ok_dialog.add_response("ok", "OK");
                                ok_dialog.present(Some(&parent_win));
                            }
                            Err(e) => {
                                let err_dialog = adw::AlertDialog::new(
                                    Some("Ошибка импорта JSON"),
                                    Some(&format!("Не удалось обработать JSON: {e}"))
                                );
                                err_dialog.add_response("ok", "Закрыть");
                                err_dialog.present(Some(&parent_win));
                            }
                        }
                    }
                }
            });

            dialog.present(Some(&win));
        }
    });

    import_row.add_suffix(&import_btn);
    json_group.add(&import_row);

    // Export Row with Button
    let copy_row = adw::ActionRow::builder()
        .title("Экспорт FastFlags")
        .subtitle("Скопировать текущие флаги в буфер обмена")
        .build();

    let copy_btn = gtk4::Button::with_label("Скопировать текущие флаги");
    copy_btn.set_valign(gtk4::Align::Center);

    let win_weak_cp = window.downgrade();
    let paths_cp = paths.clone();
    copy_btn.connect_clicked(move |_| {
        if let Some(win) = win_weak_cp.upgrade() {
            match FastFlags::export_json(&paths_cp) {
                Ok(json_str) => {
                    if let Some(display) = gtk4::gdk::Display::default() {
                        display.clipboard().set_text(&json_str);
                    }
                    let ok_dialog = adw::AlertDialog::new(
                        Some("Флаги скопированы"),
                        Some("Текущие FastFlags успешно скопированы в буфер обмена в формате JSON.")
                    );
                    ok_dialog.add_response("ok", "Отлично");
                    ok_dialog.present(Some(&win));
                }
                Err(e) => {
                    let err_dialog = adw::AlertDialog::new(
                        Some("Ошибка экспорта"),
                        Some(&format!("Не удалось экспортировать FastFlags: {e}"))
                    );
                    err_dialog.add_response("ok", "Закрыть");
                    err_dialog.present(Some(&win));
                }
            }
        }
    });

    copy_row.add_suffix(&copy_btn);
    json_group.add(&copy_row);

    page.add(&json_group);

    // 6. File path info
    let file_group = adw::PreferencesGroup::builder()
        .title("Конфигурация")
        .build();

    let path_row = adw::ActionRow::builder()
        .title("Файл ClientAppSettings.json")
        .subtitle(&*paths.fast_flags_file().to_string_lossy())
        .build();
    path_row.set_subtitle_selectable(true);
    file_group.add(&path_row);

    page.add(&file_group);

    page
}
