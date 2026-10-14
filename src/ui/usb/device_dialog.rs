use adw::prelude::*;
use gtk::glib;

use crate::model::UsbDevice;

/// Everything sysfs says about `device`, with copy buttons for the ID and
/// serial. Shows the values at the time it opens.
pub fn new(device: &UsbDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .build();
    let groups = [
        (
            None,
            vec![
                ("Vendor", device.subtitle(), false),
                ("ID", device.id(), true),
                ("Serial Number", device.serial(), true),
            ],
        ),
        (
            Some("Connection"),
            vec![
                ("Speed", device.speed(), false),
                ("USB Version", device.version(), false),
                ("Port", device.name(), false),
                ("Maximum Power", device.max_power(), false),
            ],
        ),
        (
            Some("Function"),
            vec![
                ("Class", device.class_name(), false),
                ("Driver", device.driver(), false),
            ],
        ),
    ];
    let page = adw::PreferencesPage::new();
    for (title, rows) in groups {
        let group = adw::PreferencesGroup::new();
        if let Some(title) = title {
            group.set_title(title);
        }
        let mut empty = true;
        for (label, value, copy) in rows {
            if !value.is_empty() {
                group.add(&property_row(&dialog, label, &value, copy));
                empty = false;
            }
        }
        if !empty {
            page.add(&group);
        }
    }
    dialog.add(&page);
    dialog
}

fn property_row(
    dialog: &adw::PreferencesDialog,
    label: &str,
    value: &str,
    copy: bool,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(label)
        .subtitle(value)
        .subtitle_selectable(true)
        .use_markup(false)
        .css_classes(["property"])
        .build();
    if copy {
        let button = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text("Copy")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let value = value.to_owned();
        button.connect_clicked(glib::clone!(
            #[weak]
            dialog,
            move |button| {
                button.clipboard().set_text(&value);
                dialog.add_toast(adw::Toast::new("Copied"));
            }
        ));
        row.add_suffix(&button);
    }
    row
}
