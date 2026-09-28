pub mod flags;
pub mod info;
pub mod play;
pub mod settings;

use std::sync::Arc;
use adw::prelude::*;
use gtk4::prelude::*;
use crate::paths::Paths;

pub const APP_ID: &str = "xyz.narez.MacOBlox";

pub fn open_uri(uri: &str) {
    let _ = gio::AppInfo::launch_default_for_uri(uri, gio::AppLaunchContext::NONE);
}

pub fn run_gui(paths: Arc<Paths>) -> i32 {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .build();

    let paths_clone = paths.clone();
    app.connect_activate(move |app| {
        build_window(app, paths_clone.clone());
    });

    app.run_with_args::<&str>(&[]).into()
}

fn build_window(app: &adw::Application, paths: Arc<Paths>) {
    // Add custom branding icons to the theme search path
    if let Some(display) = gtk4::gdk::Display::default() {
        let theme = gtk4::IconTheme::for_display(&display);
        theme.add_search_path(paths.project_dir.join("branding").join("icons"));
        theme.add_search_path(paths.project_dir.join("launcher").join("icons"));
        theme.add_search_path(paths.data_dir.join("branding").join("icons"));
        theme.add_search_path(paths.data_dir.join("launcher").join("icons"));
    }
    gtk4::Window::set_default_icon_name("crabblox");

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Crabblox")
        .default_width(560)
        .default_height(680)
        .resizable(false)
        .build();

    let view_stack = adw::ViewStack::new();

    let play_page = play::build_play_page(&window, paths.clone());
    let flags_page = flags::build_flags_page(&window, paths.clone());
    let settings_page = settings::build_settings_page(&window, paths.clone());
    let info_page = info::build_info_page(&window, paths.clone());

    view_stack.add_titled_with_icon(&play_page, Some("play"), "Играть", "media-playback-start-symbolic");
    view_stack.add_titled_with_icon(&flags_page, Some("flags"), "Фастфлаги", "preferences-other-symbolic");
    view_stack.add_titled_with_icon(&settings_page, Some("settings"), "Настройки", "emblem-system-symbolic");
    view_stack.add_titled_with_icon(&info_page, Some("info"), "Инфо", "help-about-symbolic");

    let switcher = adw::ViewSwitcher::builder()
        .stack(&view_stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    main_box.append(&header);
    main_box.append(&view_stack);

    window.set_content(Some(&main_box));
    window.present();
}
