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

/// UPower warning levels (`org.freedesktop.UPower.Device.WarningLevel`).
pub mod warning {
    pub const LOW: u32 = 3;
    pub const CRITICAL: u32 = 4;
    pub const ACTION: u32 = 5;
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

/// The state under the battery bar ("Not charging", "3 h 10 min left"),
/// from UPower's display device and `OnBattery`. The percentage sits beside
/// it on its own.
pub fn status_text(on_battery: bool, state: u32, time_to_empty: i64, time_to_full: i64) -> String {
    if on_battery {
        return match time_to_empty {
            0 => "On battery".into(),
            t => format!("{} left", format_duration(t)),
        };
    }
    match state {
        state::FULLY_CHARGED => "Fully charged".into(),
        state::CHARGING if time_to_full > 0 => {
            format!("Charging · full in {}", format_duration(time_to_full))
        }
        state::CHARGING => "Charging".into(),
        _ => "Not charging".into(),
    }
}

/// Some devices report only a coarse level (UPower `BatteryLevel`); give it
/// a word, or `None` when the device reports a percentage instead.
pub fn coarse_level(battery_level: u32) -> Option<&'static str> {
    match battery_level {
        3 => Some("Low"),
        4 => Some("Critical"),
        6 => Some("Normal"),
        7 => Some("High"),
        8 => Some("Full"),
        _ => None,
    }
}

/// The level as shown beside a bar: a percentage, or the coarse word.
pub fn level_text(percentage: f64, battery_level: u32) -> String {
    coarse_level(battery_level).map_or_else(|| percent(percentage), str::to_owned)
}

/// Where the bar sits, 0 to 1. Coarse levels get a stand-in position so
/// the bar still reads right and turns the warning colour when low.
pub fn level_fraction(percentage: f64, battery_level: u32) -> f64 {
    match battery_level {
        3 => 0.1,
        4 => 0.05,
        6 => 0.5,
        7 => 0.75,
        8 => 1.0,
        _ => percentage.clamp(0.0, 100.0) / 100.0,
    }
}

pub fn technology_name(technology: u32) -> &'static str {
    match technology {
        1 => "Lithium-ion",
        2 => "Lithium polymer",
        3 => "Lithium iron phosphate",
        4 => "Lead acid",
        5 => "Nickel cadmium",
        6 => "Nickel metal hydride",
        _ => "",
    }
}

/// `78.4` gives `78 %`, spaced as GNOME Settings shows it.
pub fn percent(percentage: f64) -> String {
    format!("{percentage:.0}%")
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

/// The USB device (sysfs name such as `3-2`) that owns a kernel battery,
/// from the battery's resolved sysfs `device` path. That path runs through
/// one of the USB device's interface folders (`3-2:1.2`); the last such
/// segment names the device itself, never a hub above it, since devices
/// behind a hub sit under the hub's device folder, not its interfaces.
pub fn usb_owner(device_path: &Path) -> Option<String> {
    device_path.iter().rev().find_map(|segment| {
        let (name, interface) = segment.to_str()?.split_once(':')?;
        let usb_name = name.split_once('-').is_some_and(|(bus, ports)| {
            !bus.is_empty()
                && bus.chars().all(|c| c.is_ascii_digit())
                && !ports.is_empty()
                && ports.chars().all(|c| c.is_ascii_digit() || c == '.')
        });
        let interface_id = interface.split_once('.').is_some_and(|(config, number)| {
            !config.is_empty()
                && config.chars().all(|c| c.is_ascii_digit())
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        });
        (usb_name && interface_id).then(|| name.to_owned())
    })
}

/// What a USB row says about the batteries behind it: the lowest level
/// (a receiver can serve a mouse and a keyboard), and one line per battery
/// for the details dialog. Each entry is (name, level text, bar fraction),
/// as `PowerDevice` holds them.
pub fn usb_battery_summary(batteries: &[(String, String, f64)]) -> (String, Vec<String>) {
    let row = batteries
        .iter()
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map_or_else(String::new, |(_, text, _)| format!("{text} Battery"));
    let details = batteries
        .iter()
        .map(|(name, text, _)| format!("{name} · {text}"))
        .collect();
    (row, details)
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
    fn status_texts() {
        assert_eq!(
            status_text(true, state::DISCHARGING, 11_400, 0),
            "3 h 10 min left"
        );
        assert_eq!(status_text(true, state::DISCHARGING, 0, 0), "On battery");
        assert_eq!(
            status_text(false, state::CHARGING, 0, 2_700),
            "Charging · full in 45 min"
        );
        assert_eq!(status_text(false, state::CHARGING, 0, 0), "Charging");
        assert_eq!(
            status_text(false, state::PENDING_CHARGE, 0, 0),
            "Not charging"
        );
        assert_eq!(
            status_text(false, state::FULLY_CHARGED, 0, 0),
            "Fully charged"
        );
        assert_eq!(percent(78.4), "78%");
    }

    #[test]
    fn coarse_levels() {
        assert_eq!(level_text(85.2, 1), "85%");
        assert_eq!(level_text(0.0, 7), "High");
        assert_eq!(level_text(0.0, 4), "Critical");
        assert_eq!(level_fraction(85.0, 1), 0.85);
        assert_eq!(level_fraction(0.0, 3), 0.1);
        assert_eq!(technology_name(2), "Lithium polymer");
        assert_eq!(technology_name(0), "");
    }

    #[test]
    fn usb_owners() {
        let receiver = Path::new(
            "/sys/devices/pci0000:00/0000:00:14.0/usb3/3-2/3-2:1.2/0003:046D:C52B.0003/0003:046D:4082.0004",
        );
        assert_eq!(usb_owner(receiver).as_deref(), Some("3-2"));
        let behind_hub = Path::new(
            "/sys/devices/pci0000:00/0000:00:14.0/usb3/3-1/3-1.4/3-1.4:1.0/0003:3434:0D60.0009",
        );
        assert_eq!(usb_owner(behind_hub).as_deref(), Some("3-1.4"));
        let bluetooth = Path::new("/sys/devices/virtual/misc/uhid/0005:046D:B034.000B");
        assert_eq!(usb_owner(bluetooth), None);
        // HID ids look a bit like interfaces ("0003:046D…") but aren't.
        let hid_only = Path::new("/sys/devices/platform/0003:046D:C52B.0003");
        assert_eq!(usb_owner(hid_only), None);
    }

    #[test]
    fn usb_battery_summaries() {
        let batteries = [
            ("MX Keys".to_owned(), "64%".to_owned(), 0.64),
            ("MX Master 3S".to_owned(), "30%".to_owned(), 0.30),
        ];
        let (row, details) = usb_battery_summary(&batteries);
        assert_eq!(row, "30% Battery");
        assert_eq!(details, ["MX Keys · 64%", "MX Master 3S · 30%"]);
        let coarse = [("Pad".to_owned(), level_text(0.0, 4), level_fraction(0.0, 4))];
        assert_eq!(usb_battery_summary(&coarse).0, "Critical Battery");
        assert_eq!(usb_battery_summary(&[]), (String::new(), vec![]));
    }

    #[test]
    fn connections() {
        let bluez = "/org/bluez/hci0/dev_02_00_00_00_00_02";
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
                serial: "0a:00:00:00:00:01",
                bluez: false,
            },
            Entry {
                path: "/up/mouse_dev_D5_79",
                serial: "0A:00:00:00:00:01",
                bluez: true,
            },
            Entry {
                path: "/up/keyboard_dev_E7",
                serial: "02:00:00:00:00:02",
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
