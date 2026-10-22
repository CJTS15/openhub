use std::cell::RefCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::sysinfo::{self, About};

/// GNOME Settings › System pages Openhub doesn't rebuild: (title, panel).
const SETTINGS: [(&str, &str); 6] = [
    ("Device Name", "system/about"),
    ("Region & Language", "system/region"),
    ("Date & Time", "system/datetime"),
    ("Users", "system/users"),
    ("Remote Desktop", "system/remote-desktop"),
    ("Secure Shell", "system/secure-shell"),
];

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct SystemPage {
        pub page: adw::PreferencesPage,
        pub about: RefCell<Option<About>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SystemPage {
        const NAME: &'static str = "OpenhubSystemPage";
        type Type = super::SystemPage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for SystemPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_child(Some(&self.page));
            glib::spawn_future_local(glib::clone!(
                #[weak]
                obj,
                async move {
                    let about = sysinfo::load().await;
                    obj.show(&about);
                    obj.imp().about.replace(Some(about));
                }
            ));
        }
    }

    impl WidgetImpl for SystemPage {}
    impl BinImpl for SystemPage {}
}

glib::wrapper! {
    /// The System page: this computer, as GNOME Settings › System › About
    /// shows it, and links to the System settings Openhub leaves to GNOME.
    pub struct SystemPage(ObjectSubclass<imp::SystemPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SystemPage {
    fn show(&self, about: &About) {
        let page = &self.imp().page;
        let windowing = sysinfo::windowing(&self.display());
        page.add(&self.header(about));

        let hardware = values_group("Hardware", &about.hardware());
        let copy = gtk::Button::builder()
            .label("Copy Details")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let report = about.report(&windowing);
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&report);
            button.set_label("Copied");
        });
        hardware.set_header_suffix(Some(&copy));
        page.add(&hardware);
        page.add(&values_group("Software", &about.software(&windowing)));

        // GNOME Settings comes with GNOME; elsewhere these rows would lead
        // nowhere.
        if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("GNOME")) {
            let group = adw::PreferencesGroup::builder()
                .title("More in GNOME Settings")
                .build();
            for (title, panel) in SETTINGS {
                let row = adw::ActionRow::builder()
                    .title(title)
                    .use_markup(false)
                    .activatable(true)
                    .build();
                row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
                row.connect_activated(move |row| crate::ui::open_settings_panel(row, panel));
                group.add(&row);
            }
            page.add(&group);
        }
    }

    /// The distro logo (or the computer's icon), its name and model.
    fn header(&self, about: &About) -> adw::PreferencesGroup {
        let theme = gtk::IconTheme::for_display(&self.display());
        let icon = about
            .logo
            .clone()
            .filter(|logo| theme.has_icon(logo))
            .unwrap_or_else(|| about.icon.clone());
        let image = gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(128)
            .margin_bottom(12)
            .build();
        let name = gtk::Label::builder()
            .label(&about.name)
            .css_classes(["title-1"])
            .wrap(true)
            .selectable(true)
            .build();
        let model = gtk::Label::builder()
            .label(&about.model)
            .css_classes(["dim-label"])
            .wrap(true)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        content.append(&image);
        content.append(&name);
        content.append(&model);
        let group = adw::PreferencesGroup::new();
        group.add(&content);
        group
    }
}

/// A group of rows with each value on the right, selectable for copying.
fn values_group(title: &str, rows: &[(&str, String)]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    for (name, value) in rows {
        let row = adw::ActionRow::builder()
            .title(*name)
            .use_markup(false)
            .build();
        let label = gtk::Label::builder()
            .label(value)
            .css_classes(["dim-label"])
            .selectable(true)
            .wrap(true)
            .hexpand(true)
            .xalign(1.0)
            .justify(gtk::Justification::Right)
            .build();
        row.add_suffix(&label);
        group.add(&row);
    }
    group
}
