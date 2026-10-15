//! Finds mice, touchpads and keyboards in `/sys/class/input`, which the
//! Flatpak sandbox exposes read-only. One physical device often makes
//! several input nodes (a keyboard's media keys appear as a "Mouse"), so
//! nodes are grouped per device first and the group decides what it is.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

pub const ROOT: &str = "/sys/class/input";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    // Highest first: a group with a keyboard node is a keyboard.
    Keyboard,
    Touchpad,
    PointingStick,
    Mouse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputInfo {
    /// Stable while the device is connected: its Bluetooth address, or its
    /// physical path without the `/inputN` part.
    pub key: String,
    pub name: String,
    pub kind: Kind,
    /// "USB", "Bluetooth", "Built-in" or "Other".
    pub connection: &'static str,
    /// For Bluetooth devices, the address UPower reports as the serial.
    pub uniq: String,
    /// The input node's directory, to find its USB device later.
    pub node: String,
}

/// Every mouse, touchpad, pointing stick and keyboard under `root`, one
/// per physical device, sorted by name.
pub fn scan(root: &Path) -> io::Result<Vec<InputInfo>> {
    let mut groups: BTreeMap<String, InputInfo> = BTreeMap::new();
    for entry in fs::read_dir(root)? {
        let node = entry?.file_name().to_string_lossy().into_owned();
        if !node.starts_with("input") {
            continue;
        }
        // A node unplugged mid-scan has no attributes left; skip it.
        let Some(info) = read_node(&root.join(&node), node) else {
            continue;
        };
        match groups.get(&info.key) {
            Some(kept) if kept.kind <= info.kind => {}
            _ => {
                groups.insert(info.key.clone(), info);
            }
        }
    }
    let mut devices: Vec<InputInfo> = groups.into_values().collect();
    devices.sort_by_key(|device| device.name.to_lowercase());
    Ok(devices)
}

fn read_node(dir: &Path, node: String) -> Option<InputInfo> {
    let attr = |file: &str| {
        fs::read_to_string(dir.join(file))
            .ok()
            .map(|s| s.trim().to_owned())
    };
    let kind = classify(
        &attr("capabilities/rel")?,
        &attr("capabilities/abs")?,
        &attr("capabilities/key")?,
        &attr("properties")?,
    )?;
    let bus = u16::from_str_radix(&attr("id/bustype")?, 16).ok()?;
    let connection = match bus {
        0x03 => "USB",
        0x05 => "Bluetooth",
        // i8042 (laptop keyboard and TrackPoint), I2C and the like.
        0x10 | 0x11 | 0x18 | 0x19 | 0x1f => "Built-in",
        // Virtual devices (uinput, remappers) aren't hardware.
        0x06 => return None,
        _ => "Other",
    };
    let uniq = attr("uniq").unwrap_or_default();
    let phys = attr("phys").unwrap_or_default();
    // Bluetooth nodes all carry the adapter's address as `phys`; their own
    // address is `uniq`.
    let key = if !uniq.is_empty() {
        uniq.to_lowercase()
    } else if !phys.is_empty() {
        phys.rsplit_once("/input")
            .map_or(phys.as_str(), |(base, _)| base)
            .to_owned()
    } else {
        node.clone()
    };
    let raw_name = attr("name").unwrap_or_default();
    let name = match (connection, kind) {
        ("Built-in", Kind::Keyboard) => "Built-in Keyboard".to_owned(),
        ("Built-in", Kind::Touchpad) => "Touchpad".to_owned(),
        ("Built-in", Kind::PointingStick) if raw_name.contains("TrackPoint") => {
            "TrackPoint".to_owned()
        }
        ("Built-in", Kind::PointingStick) => "Pointing Stick".to_owned(),
        _ => raw_name,
    };
    Some(InputInfo {
        key,
        name,
        kind,
        connection,
        uniq,
        node,
    })
}

/// What a node is, from its capability bitmaps, using udev's rules
/// (`input_id`): a full keyboard has keys 1 to 31, a mouse has relative
/// X/Y and a left button, a touchpad has absolute X/Y and is a pointer.
fn classify(rel: &str, abs: &str, key: &str, props: &str) -> Option<Kind> {
    const REL_X: usize = 0;
    const REL_Y: usize = 1;
    const ABS_X: usize = 0;
    const ABS_Y: usize = 1;
    const BTN_LEFT: usize = 0x110;
    const PROP_POINTER: usize = 0;
    const PROP_DIRECT: usize = 1;
    const PROP_POINTING_STICK: usize = 5;
    let pointer_motion = bit(rel, REL_X) && bit(rel, REL_Y);
    if bit(props, PROP_POINTING_STICK) && pointer_motion {
        Some(Kind::PointingStick)
    } else if bit(abs, ABS_X)
        && bit(abs, ABS_Y)
        && bit(props, PROP_POINTER)
        && !bit(props, PROP_DIRECT)
    {
        Some(Kind::Touchpad)
    } else if pointer_motion && bit(key, BTN_LEFT) {
        Some(Kind::Mouse)
    } else if (1..32).all(|n| bit(key, n)) {
        Some(Kind::Keyboard)
    } else {
        None
    }
}

/// Bit `n` of a sysfs bitmap: hex words, most significant first, each one
/// a 64-bit `unsigned long` on the platforms Openhub runs on.
fn bit(bitmap: &str, n: usize) -> bool {
    let word = bitmap
        .split_whitespace()
        .rev()
        .nth(n / 64)
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .unwrap_or(0);
    word & (1 << (n % 64)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<InputInfo> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-input");
        scan(&root).unwrap()
    }

    fn named(name: &str) -> InputInfo {
        fixture()
            .into_iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
    }

    #[test]
    fn one_entry_per_physical_device() {
        let names: Vec<_> = fixture().into_iter().map(|d| d.name).collect();
        assert_eq!(
            names,
            [
                "Built-in Keyboard",
                "Logitech MX Master 3S",
                "SmartCloud AL68 Keyboard",
                "Touchpad",
                "TrackPoint"
            ]
        );
    }

    #[test]
    fn a_keyboards_mouse_node_stays_a_keyboard() {
        let al68 = named("SmartCloud AL68 Keyboard");
        assert_eq!((al68.kind, al68.connection), (Kind::Keyboard, "USB"));
        assert_eq!(al68.key, "usb-0000:00:14.0-9");
    }

    #[test]
    fn touchpad_beats_its_mouse_node() {
        let touchpad = named("Touchpad");
        assert_eq!(
            (touchpad.kind, touchpad.connection),
            (Kind::Touchpad, "Built-in")
        );
    }

    #[test]
    fn bluetooth_mouse_is_keyed_by_its_own_address() {
        let mx = named("Logitech MX Master 3S");
        assert_eq!((mx.kind, mx.connection), (Kind::Mouse, "Bluetooth"));
        assert_eq!(mx.key, "d5:79:24:d9:f8:be");
        assert_eq!(named("TrackPoint").kind, Kind::PointingStick);
        assert_eq!(named("Built-in Keyboard").kind, Kind::Keyboard);
    }

    #[test]
    fn bits_count_from_the_last_word() {
        assert!(bit("70000 0 0 0 0", 0x110));
        assert!(!bit("70000 0 0 0 0", 0x10));
        assert!(bit("fffffffffffffffe", 1) && !bit("fffffffffffffffe", 0));
        assert!(!bit("", 3));
    }
}
