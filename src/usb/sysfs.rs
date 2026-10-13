//! Reads USB devices from sysfs, which the Flatpak sandbox exposes read-only.
//! Returns plain data, no GObjects, so tests can run it against fixture trees.

use std::fs;
use std::io;
use std::path::Path;

pub const ROOT: &str = "/sys/bus/usb/devices";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbInfo {
    /// Sysfs name such as `3-1.2`: bus 3, port 1, then port 2 of the hub there.
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub class: u8,
    pub interfaces: Vec<Interface>,
    /// The firmware marks the port as built in (`removable` is `fixed`).
    pub internal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    pub class: u8,
    pub protocol: u8,
    /// Interface string, such as `MTP` on phones.
    pub name: Option<String>,
}

impl UsbInfo {
    /// Sysfs name of the hub this device is plugged into, or `None` on a root port.
    pub fn parent(&self) -> Option<&str> {
        self.name.rsplit_once('.').map(|(parent, _)| parent)
    }

    /// Port numbers in order, for sorting: `3-1.2` gives `[3, 1, 2]`.
    fn ports(&self) -> Vec<u32> {
        self.name
            .split(['-', '.'])
            .filter_map(|port| port.parse().ok())
            .collect()
    }
}

/// Every device under `root`, hubs before what's plugged into them. Root hubs
/// (`usb1`) and interfaces (`1-1:1.0`) are left out.
pub fn scan(root: &Path) -> io::Result<Vec<UsbInfo>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir(root)? {
        let name = entry?.file_name().to_string_lossy().into_owned();
        if name.starts_with("usb") || name.contains(':') {
            continue;
        }
        // A device unplugged mid-scan has no attributes left; skip it.
        if let Some(device) = read_device(&root.join(&name), name) {
            devices.push(device);
        }
    }
    devices.sort_by_key(UsbInfo::ports);
    Ok(devices)
}

fn read_device(dir: &Path, name: String) -> Option<UsbInfo> {
    let prefix = format!("{name}:");
    let interfaces = fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| {
            let dir = entry.path();
            Some(Interface {
                class: hex(&dir, "bInterfaceClass")?,
                protocol: hex(&dir, "bInterfaceProtocol").unwrap_or_default(),
                name: attr(&dir, "interface"),
            })
        })
        .collect();
    Some(UsbInfo {
        vendor_id: hex(dir, "idVendor")?,
        product_id: hex(dir, "idProduct")?,
        manufacturer: attr(dir, "manufacturer"),
        product: attr(dir, "product"),
        class: hex(dir, "bDeviceClass").unwrap_or_default(),
        interfaces,
        internal: attr(dir, "removable").as_deref() == Some("fixed"),
        name,
    })
}

/// A sysfs attribute, trimmed; `None` when missing or empty.
fn attr(dir: &Path, file: &str) -> Option<String> {
    let value = fs::read_to_string(dir.join(file)).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn hex<T: TryFrom<u32>>(dir: &Path, file: &str) -> Option<T> {
    T::try_from(u32::from_str_radix(&attr(dir, file)?, 16).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<UsbInfo> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-usb");
        scan(&root).unwrap()
    }

    #[test]
    fn skips_root_hubs_and_interfaces_and_sorts_by_port() {
        let names: Vec<_> = fixture().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["1-1", "1-1.1", "1-1.2", "1-2"]);
    }

    #[test]
    fn reads_a_hub_and_its_children() {
        let devices = fixture();
        let hub = &devices[0];
        assert_eq!(
            (hub.vendor_id, hub.product_id, hub.class),
            (0x05e3, 0x0610, 0x09)
        );
        assert_eq!(hub.product.as_deref(), Some("USB2.1 Hub"));
        assert_eq!(hub.parent(), None);
        assert_eq!(devices[1].parent(), Some("1-1"));
        assert_eq!(devices[2].parent(), Some("1-1"));
    }

    #[test]
    fn reads_interfaces() {
        let devices = fixture();
        let phone = &devices[1];
        assert_eq!(phone.interfaces[0].name.as_deref(), Some("MTP"));
        let stick = &devices[2];
        assert_eq!(
            (stick.interfaces[0].class, stick.interfaces[0].protocol),
            (0x08, 0x50)
        );
    }

    #[test]
    fn device_without_strings_is_internal() {
        let mouse = &fixture()[3];
        assert_eq!(
            (mouse.manufacturer.as_ref(), mouse.product.as_ref()),
            (None, None)
        );
        assert!(mouse.internal);
        assert!(!fixture()[0].internal);
    }
}
