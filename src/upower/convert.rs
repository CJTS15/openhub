//! Plain rules for turning UPower values into what the Power page shows. No
//! GObjects or D-Bus, so they're tested without a bus.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// UPower device types (`org.freedesktop.UPower.Device.Type`) Openhub names.
pub mod kind {
    pub const LINE_POWER: u32 = 1;
    pub const BATTERY: u32 = 2;
    pub const UPS: u32 = 3;
    pub const MOUSE: u32 = 5;
    pub const KEYBOARD: u32 = 6;
    pub const PHONE: u32 = 8;
    pub const TABLET: u32 = 10;
    pub const COMPUTER: u32 = 11;
    pub const GAMING_INPUT: u32 = 12;
    pub const PEN: u32 = 13;
    pub const TOUCHPAD: u32 = 14;
    pub const HEADSET: u32 = 17;
    pub const SPEAKERS: u32 = 18;
    pub const HEADPHONES: u32 = 19;
}

/// UPower charge states (`org.freedesktop.UPower.Device.State`).
pub mod state {
    pub const CHARGING: u32 = 1;
    pub const DISCHARGING: u32 = 2;
    pub const EMPTY: u32 = 3;
    pub const FULLY_CHARGED: u32 = 4;
    pub const PENDING_CHARGE: u32 = 5;
    pub const PENDING_DISCHARGE: u32 = 6;
}

/// A name for devices that don't report a model.
pub fn kind_name(kind: u32) -> &'static str {
    match kind {
        kind::LINE_POWER => "Charger",
        kind::BATTERY => "Battery",
        kind::UPS => "UPS",
        kind::MOUSE => "Mouse",
        kind::KEYBOARD => "Keyboard",
        kind::PHONE => "Phone",
        kind::TABLET => "Tablet",
        kind::COMPUTER => "Computer",
        kind::GAMING_INPUT => "Game Controller",
        kind::PEN => "Pen",
        kind::TOUCHPAD => "Touchpad",
        kind::HEADSET => "Headset",
        kind::SPEAKERS => "Speakers",
        kind::HEADPHONES => "Headphones",
        _ => "Device",
    }
}

pub fn kind_icon(kind: u32) -> &'static str {
    match kind {
        kind::LINE_POWER => "ac-adapter-symbolic",
        kind::UPS => "uninterruptible-power-supply-symbolic",
        kind::MOUSE => "input-mouse-symbolic",
        kind::KEYBOARD => "input-keyboard-symbolic",
        kind::PHONE => "phone-symbolic",
        kind::TABLET | kind::PEN => "input-tablet-symbolic",
        kind::COMPUTER => "computer-symbolic",
        kind::GAMING_INPUT => "input-gaming-symbolic",
        kind::TOUCHPAD => "input-touchpad-symbolic",
        kind::HEADSET => "audio-headset-symbolic",
        kind::SPEAKERS => "audio-speakers-symbolic",
        kind::HEADPHONES => "audio-headphones-symbolic",
        _ => "battery-symbolic",
    }
}

/// Empty when UPower doesn't know, which peripherals often report.
pub fn state_label(state: u32) -> &'static str {
    match state {
        state::CHARGING => "Charging",
        state::DISCHARGING => "Discharging",
        state::EMPTY => "Empty",
        state::FULLY_CHARGED => "Fully charged",
        // Plugged in but held, for example by a charge limit.
        state::PENDING_CHARGE => "Not charging",
        state::PENDING_DISCHARGE => "Waiting to discharge",
        _ => "",
    }
}

/// `11400` gives `3 h 10 min`; under a minute gives `less than a minute`.
pub fn format_duration(seconds: i64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "less than a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

/// The page's top line, from UPower's display device and `OnBattery`.
pub fn status_line(
    on_battery: bool,
    percentage: f64,
    state: u32,
    time_to_empty: i64,
    time_to_full: i64,
) -> String {
    let level = format!("{percentage:.0}%");
    if on_battery {
        return match time_to_empty {
            0 => format!("On battery · {level}"),
            t => format!("On battery · {level} · {} left", format_duration(t)),
        };
    }
    match state {
        state::FULLY_CHARGED => "Plugged in · Fully charged".into(),
        state::CHARGING if time_to_full > 0 => format!(
            "Plugged in · Charging · {level} · full in {}",
            format_duration(time_to_full)
        ),
        state::CHARGING => format!("Plugged in · Charging · {level}"),
        _ => format!("Plugged in · {level} · Not charging"),
    }
}

/// How a peripheral is connected. UPower reports Bluetooth batteries
/// either through BlueZ (`native_path` is a BlueZ object path) or through
/// the kernel's HID driver, whose power supply sits under `uhid`;
/// `device_path` is the power supply's resolved sysfs `device` link.
pub fn connection(native_path: &str, device_path: Option<&Path>) -> &'static str {
    if native_path.starts_with("/org/bluez/") {
        return "Bluetooth";
    }
    let path = device_path.map(|p| p.to_string_lossy()).unwrap_or_default();
    if path.contains("/uhid/") || path.contains("/bluetooth/") {
        "Bluetooth"
    } else if path.contains("/usb") {
        "USB"
    } else {
        "Other"
    }
}

/// One peripheral, as far as merging duplicates needs to know.
pub struct Entry<'a> {
    pub path: &'a str,
    pub serial: &'a str,
    pub bluez: bool,
}

/// UPower can report one Bluetooth device twice, through BlueZ and through
/// the kernel, with the same serial (its address, in either case). Returns
/// the paths to hide: every entry sharing a serial except one, preferring
/// the BlueZ one.
pub fn hidden_duplicates(entries: &[Entry]) -> HashSet<String> {
    let mut by_serial: HashMap<String, Vec<&Entry>> = HashMap::new();
    for entry in entries.iter().filter(|e| !e.serial.is_empty()) {
        by_serial
            .entry(entry.serial.to_lowercase())
            .or_default()
            .push(entry);
    }
    let mut hidden = HashSet::new();
    for mut group in by_serial.into_values().filter(|g| g.len() > 1) {
        // Keep the BlueZ entry, else the first by path so the choice is stable.
        group.sort_by_key(|e| (!e.bluez, e.path));
        hidden.extend(group[1..].iter().map(|e| e.path.to_owned()));
    }
    hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration(11_400), "3 h 10 min");
        assert_eq!(format_duration(2_700), "45 min");
        assert_eq!(format_duration(7_200), "2 h");
        assert_eq!(format_duration(30), "less than a minute");
    }

    #[test]
    fn status_lines() {
        assert_eq!(
            status_line(true, 64.4, state::DISCHARGING, 11_400, 0),
            "On battery · 64% · 3 h 10 min left"
        );
        assert_eq!(
            status_line(true, 64.0, state::DISCHARGING, 0, 0),
            "On battery · 64%"
        );
        assert_eq!(
            status_line(false, 50.0, state::CHARGING, 0, 2_700),
            "Plugged in · Charging · 50% · full in 45 min"
        );
        assert_eq!(
            status_line(false, 78.0, state::PENDING_CHARGE, 0, 0),
            "Plugged in · 78% · Not charging"
        );
        assert_eq!(
            status_line(false, 100.0, state::FULLY_CHARGED, 0, 0),
            "Plugged in · Fully charged"
        );
    }

    #[test]
    fn connections() {
        let bluez = "/org/bluez/hci0/dev_E7_1E_F6_9D_9E_4A";
        assert_eq!(connection(bluez, None), "Bluetooth");
        let uhid = Path::new("/sys/devices/virtual/misc/uhid/0005:046D:B034.000B");
        assert_eq!(connection("hidpp_battery_0", Some(uhid)), "Bluetooth");
        let usb = Path::new(
            "/sys/devices/pci0000:00/0000:00:14.0/usb3/3-2/3-2:1.2/0003:046D:C52B.0003/0003:046D:4082.0004",
        );
        assert_eq!(connection("hidpp_battery_1", Some(usb)), "USB");
        assert_eq!(connection("ps-controller-battery", None), "Other");
    }

    #[test]
    fn keeps_the_bluez_duplicate() {
        let entries = [
            Entry {
                path: "/up/battery_hidpp_battery_0",
                serial: "d5:79:24:d9:f8:be",
                bluez: false,
            },
            Entry {
                path: "/up/mouse_dev_D5_79",
                serial: "D5:79:24:D9:F8:BE",
                bluez: true,
            },
            Entry {
                path: "/up/keyboard_dev_E7",
                serial: "E7:1E:F6:9D:9E:4A",
                bluez: true,
            },
            Entry {
                path: "/up/no_serial_a",
                serial: "",
                bluez: false,
            },
            Entry {
                path: "/up/no_serial_b",
                serial: "",
                bluez: false,
            },
        ];
        assert_eq!(
            hidden_duplicates(&entries),
            HashSet::from(["/up/battery_hidpp_battery_0".to_owned()])
        );
    }

    #[test]
    fn duplicates_without_bluez_keep_one() {
        let entries = [
            Entry {
                path: "/up/b",
                serial: "AA",
                bluez: false,
            },
            Entry {
                path: "/up/a",
                serial: "aa",
                bluez: false,
            },
        ];
        assert_eq!(
            hidden_duplicates(&entries),
            HashSet::from(["/up/b".to_owned()])
        );
    }
}
