use adw::prelude::*;

use crate::i18n::{gettext, gettext_f, gettext_noop};
use crate::model::PowerDevice;
use crate::upower::convert::percent;

/// What UPower knows about a battery, at the time the dialog opens.
/// Values a device doesn't report are left out, so peripherals show little.
pub fn new(device: &PowerDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .build();
    // Translators: an amount of energy, such as "52.4 Wh".
    let wh = |value: f64| (value > 0.0).then(|| gettext_f("{} Wh", &[&format!("{value:.1}")]));
    let groups = [
        (
            None,
            vec![
                (
                    gettext_noop("Level"),
                    Some(device.level_text()).filter(|t| !t.is_empty()),
                ),
                (
                    gettext_noop("Health"),
                    (device.capacity() > 0.0).then(|| percent(device.capacity())),
                ),
                (
                    gettext_noop("Charge Cycles"),
                    (device.charge_cycles() >= 0).then(|| device.charge_cycles().to_string()),
                ),
                (
                    gettext_noop("Technology"),
                    Some(device.technology()).filter(|t| !t.is_empty()),
                ),
            ],
        ),
        (
            Some(gettext_noop("Energy")),
            vec![
                (gettext_noop("Current Charge"), wh(device.energy())),
                (gettext_noop("Full Charge"), wh(device.energy_full())),
                (
                    gettext_noop("Design Capacity"),
                    wh(device.energy_full_design()),
                ),
                (
                    gettext_noop("Energy Rate"),
                    (device.power_supply() && device.energy_full() > 0.0)
                        .then(|| format!("{:.1} W", device.energy_rate())),
                ),
                (
                    gettext_noop("Voltage"),
                    (device.voltage() > 0.0)
                        .then(|| gettext_f("{} V", &[&format!("{:.1}", device.voltage())])),
                ),
            ],
        ),
        (
            Some(gettext_noop("Device")),
            vec![
                (gettext_noop("Vendor"), Some(device.vendor())),
                (gettext_noop("Model"), Some(device.model())),
                (gettext_noop("Serial Number"), Some(device.serial())),
                (gettext_noop("Connection"), Some(device.connection())),
            ],
        ),
    ];
    let page = adw::PreferencesPage::new();
    for (title, rows) in groups {
        let group = adw::PreferencesGroup::new();
        if let Some(title) = title {
            group.set_title(&gettext(title));
        }
        let mut empty = true;
        for (label, value) in rows {
            let Some(value) = value.filter(|v| !v.is_empty()) else {
                continue;
            };
            group.add(
                &adw::ActionRow::builder()
                    .title(gettext(label))
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
