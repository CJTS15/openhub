use adw::prelude::*;
use gtk::glib;

use crate::fwupd::{Device, Release};

/// A device's firmware: its version, and the update the host's metadata
/// has, with its release notes and a way into GNOME Software to install.
pub fn new(device: &Device, update: Option<&Release>) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(&device.name)
        .content_width(420)
        .build();
    let page = adw::PreferencesPage::new();

    let current = adw::PreferencesGroup::new();
    for (title, value) in [("Vendor", &device.vendor), ("Version", &device.version)] {
        if !value.is_empty() {
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(value)
                .use_markup(false)
                .subtitle_selectable(true)
                .css_classes(["property"])
                .build();
            current.add(&row);
        }
    }
    page.add(&current);

    let group = adw::PreferencesGroup::new();
    match update {
        None => group.set_description(Some("This firmware is up to date.")),
        Some(release) => {
            group.set_title("Update Available");
            let row = adw::ActionRow::builder()
                .title("New Version")
                .subtitle(&release.version)
                .use_markup(false)
                .css_classes(["property"])
                .build();
            group.add(&row);
            if !release.notes.is_empty() {
                group.add(
                    &gtk::Label::builder()
                        .label(&release.notes)
                        .wrap(true)
                        .xalign(0.0)
                        .selectable(true)
                        .margin_top(12)
                        .build(),
                );
            }
            // Installing needs a download, polkit and often a reboot; GNOME
            // Software does all of that.
            let open = gtk::Button::builder()
                .label("Open GNOME Software")
                .halign(gtk::Align::Center)
                .margin_top(18)
                .css_classes(["pill", "suggested-action"])
                .visible(false)
                .build();
            open.connect_clicked(|button| {
                crate::ui::activate_app(
                    button,
                    "org.gnome.Software",
                    "set-mode",
                    "updates".to_variant(),
                );
            });
            group.add(&open);
            glib::spawn_future_local(glib::clone!(
                #[weak]
                open,
                async move {
                    open.set_visible(crate::ui::app_installed("org.gnome.Software").await);
                }
            ));
        }
    }
    page.add(&group);
    dialog.add(&page);
    dialog
}
