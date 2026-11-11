use adw::prelude::*;

use super::page::refresh;
use crate::display::{Logical, Monitor, edid};
use crate::i18n::{gettext, gettext_f, gettext_noop};

/// What a monitor is (from its EDID) and what it can do (from Mutter).
pub fn new(
    monitor: &Monitor,
    logical: Option<&Logical>,
    edid: Option<&edid::Edid>,
) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(if monitor.builtin {
            gettext("Built-in Display")
        } else {
            monitor.display_name.clone()
        })
        .content_width(560)
        .build();
    let page = adw::PreferencesPage::new();

    let model = edid
        .and_then(|e| e.name.clone().or_else(|| e.text.last().cloned()))
        .unwrap_or_else(|| monitor.product.clone());
    let maker = edid.map_or(monitor.vendor.as_str(), |e| edid::vendor_name(&e.vendor));
    // Mutter reports "0x00000000" when the EDID has no serial.
    let serial = edid.and_then(|e| e.serial.clone()).or_else(|| {
        Some(monitor.serial.clone()).filter(|s| {
            !s.trim_start_matches("0x")
                .trim_start_matches('0')
                .is_empty()
        })
    });
    let made = edid.map(|e| match e.week {
        0 => e.year.to_string(),
        // Translators: when a monitor was made, such as "Week 12, 2023".
        week => gettext_f("Week {}, {}", &[&week.to_string(), &e.year.to_string()]),
    });
    let size = edid.and_then(|e| {
        e.diagonal_inches()
            .map(|inches| format!("{} × {} cm · {inches:.1}″", e.width_cm, e.height_cm))
    });
    page.add(&group(
        Some(gettext_noop("Monitor")),
        &[
            (gettext_noop("Manufacturer"), Some(maker.to_owned())),
            (gettext_noop("Model"), Some(model)),
            (gettext_noop("Serial Number"), serial),
            (gettext_noop("Manufactured"), made),
            (gettext_noop("Size"), size),
            (gettext_noop("Connector"), Some(monitor.connector.clone())),
            (
                gettext_noop("Type"),
                Some(if monitor.builtin {
                    gettext("Built-in")
                } else {
                    gettext("External")
                }),
            ),
        ],
    ));

    let current = monitor.current_mode();
    let vrr = monitor.vrr_supported().then(|| match monitor.min_refresh {
        Some(min) => gettext_f("Supported, from {} Hz", &[&min.to_string()]),
        None => gettext("Supported"),
    });
    page.add(&group(
        Some(gettext_noop("Current Mode")),
        &[
            (
                gettext_noop("Resolution"),
                current.map(|m| format!("{} × {}", m.width, m.height)),
            ),
            (
                gettext_noop("Refresh Rate"),
                current.map(|m| gettext_f("{} Hz", &[&refresh(m.refresh)])),
            ),
            (
                gettext_noop("Scale"),
                logical.map(|l| format!("{:.0}%", l.scale * 100.0)),
            ),
            (gettext_noop("Variable Refresh Rate"), vrr),
            (
                "HDR",
                Some(if monitor.hdr_supported() {
                    gettext("Supported")
                } else {
                    gettext("Not supported")
                }),
            ),
        ],
    ));

    // One row per resolution, with its refresh rates; VRR variants are the
    // same rates again.
    let mut resolutions: Vec<((i32, i32), Vec<f64>)> = Vec::new();
    for mode in monitor.modes.iter().filter(|m| !m.variable) {
        let size = (mode.width, mode.height);
        match resolutions.iter_mut().find(|(s, _)| *s == size) {
            Some((_, rates)) => rates.push(mode.refresh),
            None => resolutions.push((size, vec![mode.refresh])),
        }
    }
    let modes = adw::PreferencesGroup::builder()
        .title(gettext("Supported Modes"))
        .build();
    for ((width, height), mut rates) in resolutions {
        rates.sort_by(|a, b| b.total_cmp(a));
        let rates: Vec<String> = rates.iter().map(|r| refresh(*r)).collect();
        modes.add(
            &adw::ActionRow::builder()
                .title(format!("{width} × {height}"))
                .subtitle(gettext_f("{} Hz", &[&rates.join(", ")]))
                .css_classes(["property"])
                .build(),
        );
    }
    page.add(&modes);
    dialog.add(&page);
    dialog
}

fn group(title: Option<&str>, rows: &[(&str, Option<String>)]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    if let Some(title) = title {
        group.set_title(&gettext(title));
    }
    for (label, value) in rows {
        if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
            group.add(
                &adw::ActionRow::builder()
                    .title(gettext(*label))
                    .subtitle(value)
                    .subtitle_selectable(true)
                    .use_markup(false)
                    .css_classes(["property"])
                    .build(),
            );
        }
    }
    group
}
