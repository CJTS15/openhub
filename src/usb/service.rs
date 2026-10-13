//! Turns a sysfs scan into the `UsbDevice` tree the USB page shows.

use std::collections::HashMap;
use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::ids::UsbIds;
use super::sysfs::{self, UsbInfo};
use crate::model::UsbDevice;

#[derive(Debug)]
pub struct UsbManager {
    devices: gio::ListStore,
}

impl Default for UsbManager {
    fn default() -> Self {
        Self {
            devices: gio::ListStore::new::<UsbDevice>(),
        }
    }
}

impl UsbManager {
    /// `UsbDevice` items on root ports; hubs hold the rest as children.
    pub fn devices(&self) -> gio::ListStore {
        self.devices.clone()
    }

    /// Rebuilds the tree from sysfs.
    pub fn refresh(&self) {
        let infos = sysfs::scan(Path::new(sysfs::ROOT)).unwrap_or_else(|err| {
            glib::g_warning!("openhub", "Can't read {}: {err}", sysfs::ROOT);
            Vec::new()
        });
        let ids = UsbIds::system();
        let mut by_name: HashMap<String, UsbDevice> = HashMap::new();
        let mut top_level = Vec::new();
        // The scan lists hubs before their children, so a parent is always known.
        for info in &infos {
            let device = new_device(info, ids);
            match info.parent().and_then(|parent| by_name.get(parent)) {
                Some(hub) => hub.children().append(&device),
                None => top_level.push(device.clone()),
            }
            by_name.insert(info.name.clone(), device);
        }
        self.devices.splice(0, self.devices.n_items(), &top_level);
    }
}

fn new_device(info: &UsbInfo, ids: &UsbIds) -> UsbDevice {
    // The device's own strings first, then usb.ids, then the raw ID.
    let vendor = info
        .manufacturer
        .as_deref()
        .or_else(|| ids.vendor(info.vendor_id));
    let product = info
        .product
        .as_deref()
        .or_else(|| ids.product(info.vendor_id, info.product_id));
    let id = format!("{:04x}:{:04x}", info.vendor_id, info.product_id);
    glib::Object::builder()
        .property("name", &info.name)
        .property("title", product.unwrap_or(&id))
        .property("subtitle", vendor.unwrap_or_default())
        .property("icon-name", icon_name(info))
        .property("internal", info.internal)
        .build()
}

/// An icon from the device class, or the first interface class that says
/// more (many devices declare "per interface").
fn icon_name(info: &UsbInfo) -> &'static str {
    let classes = std::iter::once((info.class, 0))
        .chain(info.interfaces.iter().map(|i| (i.class, i.protocol)));
    for (class, protocol) in classes {
        return match (class, protocol) {
            (0x01, _) => "audio-card-symbolic",
            (0x02 | 0x0a, _) => "network-wired-symbolic",
            (0x03, 2) => "input-mouse-symbolic",
            (0x03, _) => "input-keyboard-symbolic",
            (0x06, _) => "camera-photo-symbolic",
            (0x07, _) => "printer-symbolic",
            (0x08, _) => "drive-removable-media-symbolic",
            (0x0e, _) => "camera-web-symbolic",
            (0x11, _) => "video-display-symbolic",
            (0xe0, _) => "bluetooth-symbolic",
            _ => continue,
        };
    }
    // Phones use a vendor-specific class for MTP but name the interface.
    if info
        .interfaces
        .iter()
        .any(|i| i.name.as_deref() == Some("MTP"))
    {
        return "phone-symbolic";
    }
    "media-removable-symbolic"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_follow_device_then_interface_class() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-usb");
        let icons: Vec<_> = sysfs::scan(&root).unwrap().iter().map(icon_name).collect();
        assert_eq!(
            icons,
            [
                "media-removable-symbolic",       // hub
                "phone-symbolic",                 // MTP
                "drive-removable-media-symbolic", // mass storage
                "input-mouse-symbolic",           // HID mouse
            ]
        );
    }
}
