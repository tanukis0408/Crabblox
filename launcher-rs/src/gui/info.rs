use std::sync::Arc;
use adw::prelude::*;
use gtk4::prelude::*;
use crate::paths::Paths;

pub fn build_info_page(_window: &adw::ApplicationWindow, paths: Arc<Paths>) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();

    // About Group
    let about_group = adw::PreferencesGroup::builder()
        .title("Crabblox")
        .description("Crabblox запускает настоящий клиент Roblox для macOS на Linux через Darling.\nПроект разрабатывается Monster Dev и никак не связан с Roblox.")
        .build();
    page.add(&about_group);

    // Community Group
    let comm_group = adw::PreferencesGroup::builder()
        .title("Сообщество")
        .build();

    let discord_row = adw::ActionRow::builder()
        .title("Discord")
        .subtitle("Сервер сообщества")
        .activatable(true)
        .build();
    discord_row.add_prefix(&gtk4::Image::from_icon_name("macoblox-discord-symbolic"));
    discord_row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));

    discord_row.connect_activated(|_| {
        crate::gui::open_uri("https://discord.gg/bpX9rTttCa");
    });
    comm_group.add(&discord_row);

    let github_row = adw::ActionRow::builder()
        .title("GitHub")
        .subtitle("Исходный код проекта")
        .activatable(true)
        .build();
    github_row.add_prefix(&gtk4::Image::from_icon_name("macoblox-github-symbolic"));
    github_row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));

    github_row.connect_activated(|_| {
        crate::gui::open_uri("https://github.com/narezy/MacOBlox");
    });
    comm_group.add(&github_row);

    page.add(&comm_group);

    // Developers Group
    let dev_group = adw::PreferencesGroup::builder()
        .title("Разработчики")
        .build();

    let dev_row = adw::ActionRow::builder()
        .title("Monster Dev")
        .subtitle("Команда Monster Dev")
        .activatable(true)
        .build();

    let avatar = adw::Avatar::builder()
        .size(48)
        .text("Monster Dev")
        .show_initials(true)
        .build();

    let avatar_path = paths.project_dir.join("branding").join("icons").join("crabblox-128.png");
    if avatar_path.exists() {
        let gfile = gio::File::for_path(&avatar_path);
        if let Ok(texture) = gtk4::gdk::Texture::from_file(&gfile) {
            avatar.set_custom_image(Some(&texture));
        }
    }
    dev_row.add_prefix(&avatar);
    dev_row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));

    dev_row.connect_activated(|_| {
        crate::gui::open_uri("https://github.com/narezy/MacOBlox");
    });
    dev_group.add(&dev_row);

    page.add(&dev_group);

    // Support Group
    let support_group = adw::PreferencesGroup::builder()
        .title("Поддержать проект")
        .description("Crabblox бесплатный. Если он вам пригодился, можно поддержать авторов.")
        .build();

    let boosty_row = adw::ActionRow::builder()
        .title("Boosty")
        .subtitle("Карты любых стран")
        .activatable(true)
        .build();
    boosty_row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));
    boosty_row.connect_activated(|_| {
        crate::gui::open_uri("https://boosty.to/ega_link");
    });
    support_group.add(&boosty_row);

    let yoomoney_row = adw::ActionRow::builder()
        .title("ЮMoney")
        .subtitle("Для России")
        .activatable(true)
        .build();
    yoomoney_row.add_suffix(&gtk4::Image::from_icon_name("adw-external-link-symbolic"));
    yoomoney_row.connect_activated(|_| {
        crate::gui::open_uri("https://yoomoney.ru/to/4100118196133693");
    });
    support_group.add(&yoomoney_row);

    page.add(&support_group);

    page
}
