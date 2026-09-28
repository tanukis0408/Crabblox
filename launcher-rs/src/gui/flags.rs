use std::sync::Arc;
use adw::prelude::*;
use gtk4::prelude::*;
use crate::fast_flags::FastFlags;
use crate::paths::Paths;

pub fn build_flags_page(_window: &adw::ApplicationWindow, paths: Arc<Paths>) -> adw::PreferencesPage {
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

    // 2. Popular flags
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

    // 3. File path info
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
