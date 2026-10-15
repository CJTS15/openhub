use adw::prelude::*;

use crate::model::PowerDevice;
use crate::upower::convert::percent;

/// What UPower knows about a battery, at the time the dialog opens.
/// Values a device doesn't report are left out, so peripherals show little.
pub fn new(device: &PowerDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .build();
    let wh = |value: f64| (value > 0.0).then(|| format!("{value:.1} Wh"));
    let groups = [
        (
            None,
            vec![
                ("Level", Some(device.level_text()).filter(|t| !t.is_empty())),
                (
                    "Health",
                    (device.capacity() > 0.0).then(|| percent(device.capacity())),
                ),
                (
                    "Charge Cycles",
                    (device.charge_cycles() >= 0).then(|| device.charge_cycles().to_string()),
                ),
                (
                    "Technology",
                    Some(device.technology()).filter(|t| !t.is_empty()),
                ),
            ],
        ),
        (
            Some("Energy"),
            vec![
                ("Current Charge", wh(device.energy())),
                ("Full Charge", wh(device.energy_full())),
                ("Design Capacity", wh(device.energy_full_design())),
                (
                    "Energy Rate",
                    (device.power_supply() && device.energy_full() > 0.0)
                        .then(|| format!("{:.1} W", device.energy_rate())),
                ),
                (
                    "Voltage",
                    (device.voltage() > 0.0).then(|| format!("{:.1} V", device.voltage())),
                ),
            ],
        ),
        (
            Some("Device"),
            vec![
                ("Vendor", Some(device.vendor())),
                ("Model", Some(device.model())),
                ("Serial Number", Some(device.serial())),
                ("Connection", Some(device.connection())),
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
        for (label, value) in rows {
            let Some(value) = value.filter(|v| !v.is_empty()) else {
                continue;
            };
            group.add(
                &adw::ActionRow::builder()
                    .title(label)
                    .subtitle(value)
                    .subtitle_selectable(true)
                    .use_markup(false)
                    .css_classes(["property"])
                    .build(),
            );
            empty = false;
        }
        if !empty {
            page.add(&group);
        }
    }
    dialog.add(&page);
    dialog
}
