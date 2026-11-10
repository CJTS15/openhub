use adw::prelude::*;
use gtk::glib;

use crate::i18n::{gettext, gettext_noop};
use crate::model::UsbDevice;
use gtk::glib::clone;

/// Everything sysfs says about `device`, with copy buttons for the ID and
/// serial. Shows the values at the time it opens.
pub fn new(device: &UsbDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .build();
    let mut groups = vec![
        (
            None,
            vec![
                (gettext_noop("Vendor"), device.subtitle(), false),
                (gettext_noop("ID"), device.id(), true),
                (gettext_noop("Serial Number"), device.serial(), true),
            ],
        ),
        (
            Some(gettext_noop("Connection")),
            vec![
                (gettext_noop("Speed"), device.speed(), false),
                (gettext_noop("USB Version"), device.version(), false),
                (gettext_noop("Port"), device.name(), false),
                (gettext_noop("Maximum Power"), device.max_power(), false),
            ],
        ),
        (
            Some(gettext_noop("Function")),
            vec![
                (gettext_noop("Class"), device.class_name(), false),
                (gettext_noop("Driver"), device.driver(), false),
            ],
        ),
    ];
    // A receiver's devices' batteries, from UPower, right after the identity.
    let batteries = device
        .batteries()
        .into_iter()
        .map(|line| (gettext_noop("Battery"), line, false))
        .collect();
    groups.insert(1, (Some(gettext_noop("Battery")), batteries));
    let page = adw::PreferencesPage::new();
    let mut identity = None;
    for (title, rows) in groups {
        let group = adw::PreferencesGroup::new();
        if title.is_none() {
            identity = Some(group.clone());
        }
        if let Some(title) = title {
            group.set_title(&gettext(title));
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
    // The firmware version, where fwupd knows this device.
    if let Some(identity) = identity {
        let id = device.id();
        glib::spawn_future_local(clone!(
            #[weak]
            dialog,
            async move {
                let Ok(bus) = crate::fwupd::bus().await else {
                    return;
                };
                let devices = crate::fwupd::devices(&bus).await.unwrap_or_default();
                if let Some(found) = devices
                    .iter()
                    .find(|d| d.matches_usb(&id) && !d.version.is_empty())
                {
                    identity.add(&property_row(
                        &dialog,
                        gettext_noop("Firmware"),
                        &found.version,
                        false,
                    ));
                }
            }
        ));
    }
    dialog
}

fn property_row(
    dialog: &adw::PreferencesDialog,
    label: &str,
    value: &str,
    copy: bool,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(gettext(label))
        .subtitle(value)
        .subtitle_selectable(true)
        .use_markup(false)
        .css_classes(["property"])
        .build();
    if copy {
        let button = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(gettext("Copy"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let value = value.to_owned();
        button.connect_clicked(clone!(
            #[weak]
            dialog,
            move |button| {
                button.clipboard().set_text(&value);
                dialog.add_toast(adw::Toast::new(&gettext("Copied")));
            }
        ));
        row.add_suffix(&button);
    }
    row
}
