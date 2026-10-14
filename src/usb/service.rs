//! Turns sysfs scans into the `UsbDevice` tree the USB page shows, updating
//! it in place so rows keep their state between scans.

use std::cell::RefCell;
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
    /// Every device from the last scan, by sysfs name.
    known: RefCell<HashMap<String, UsbDevice>>,
}

impl Default for UsbManager {
    fn default() -> Self {
        Self {
            devices: gio::ListStore::new::<UsbDevice>(),
            known: RefCell::default(),
        }
    }
}

impl UsbManager {
    /// `UsbDevice` items on root ports; hubs hold the rest as children.
    pub fn devices(&self) -> gio::ListStore {
        self.devices.clone()
    }

    /// Rescans sysfs. A device on the same port with the same ID keeps its
    /// object; lists change only where devices came or went.
    pub fn refresh(&self) {
        let infos = sysfs::scan(Path::new(sysfs::ROOT)).unwrap_or_else(|err| {
            glib::g_warning!("openhub", "Can't read {}: {err}", sysfs::ROOT);
            Vec::new()
        });
        let ids = UsbIds::system();
        let mut known = self.known.borrow_mut();
        let mut next: HashMap<String, UsbDevice> = HashMap::new();
        let mut top_level = Vec::new();
        let mut children: HashMap<String, Vec<UsbDevice>> = HashMap::new();
        // The scan lists hubs before their children, so a parent is always known.
        for info in &infos {
            let id = format!("{:04x}:{:04x}", info.vendor_id, info.product_id);
            let device = known
                .remove(&info.name)
                .filter(|device| device.id() == id)
                .unwrap_or_else(glib::Object::new);
            update(&device, info, &id, ids);
            match info.parent().filter(|parent| next.contains_key(*parent)) {
                Some(parent) => children
                    .entry(parent.to_owned())
                    .or_default()
                    .push(device.clone()),
                None => top_level.push(device.clone()),
            }
            next.insert(info.name.clone(), device);
        }
        for (name, device) in &next {
            sync(
                &device.children(),
                &children.remove(name).unwrap_or_default(),
            );
        }
        sync(&self.devices, &top_level);
        *known = next;
    }
}

/// Makes `store` hold `wanted` in order, removing and inserting only what
/// changed. Both are sorted by port, so kept items are already in order.
fn sync(store: &gio::ListStore, wanted: &[UsbDevice]) {
    for position in (0..store.n_items()).rev() {
        let item = store.item(position).and_downcast::<UsbDevice>();
        if !item.is_some_and(|item| wanted.contains(&item)) {
            store.remove(position);
        }
    }
    for (position, device) in (0u32..).zip(wanted) {
        if store.item(position).as_ref() != Some(device.upcast_ref()) {
            store.insert(position, device);
        }
    }
}

fn update(device: &UsbDevice, info: &UsbInfo, id: &str, ids: &UsbIds) {
    // The device's own strings first, then usb.ids, then the raw ID.
    let vendor = info
        .manufacturer
        .as_deref()
        .or_else(|| ids.vendor(info.vendor_id));
    let product = info
        .product
        .as_deref()
        .or_else(|| ids.product(info.vendor_id, info.product_id));
    let mut drivers: Vec<&str> = info
        .interfaces
        .iter()
        .filter_map(|i| i.driver.as_deref())
        .collect();
    drivers.dedup();
    // Setting an unchanged value would still notify and redraw bound rows.
    let set = |property: &str, value: String| {
        if device.property::<String>(property) != value {
            device.set_property(property, value);
        }
    };
    set("name", info.name.clone());
    set("id", id.to_owned());
    set("title", product.unwrap_or(id).to_owned());
    set("subtitle", vendor.unwrap_or_default().to_owned());
    set("icon-name", icon_name(info).to_owned());
    set("serial", info.serial.clone().unwrap_or_default());
    set(
        "speed",
        info.speed.as_deref().map(speed_label).unwrap_or_default(),
    );
    set(
        "version",
        info.version
            .as_deref()
            .map(version_label)
            .unwrap_or_default(),
    );
    set("class-name", class_label(info));
    set("driver", drivers.join(", "));
    set("max-power", info.max_power.clone().unwrap_or_default());
    device.set_internal(info.internal);
    device.set_hub(info.class == 0x09);
}

/// `480` gives `480 Mbit/s (High Speed)`.
fn speed_label(mbps: &str) -> String {
    match mbps {
        "1.5" => "1.5 Mbit/s (Low Speed)".into(),
        "12" => "12 Mbit/s (Full Speed)".into(),
        "480" => "480 Mbit/s (High Speed)".into(),
        "5000" => "5 Gbit/s (SuperSpeed)".into(),
        "10000" => "10 Gbit/s (SuperSpeed+)".into(),
        "20000" => "20 Gbit/s (SuperSpeed+)".into(),
        other => format!("{other} Mbit/s"),
    }
}

/// `2.10` gives `USB 2.1`; `2.01` stays `USB 2.01`.
fn version_label(version: &str) -> String {
    let version = match version.strip_suffix('0') {
        Some(short) if !short.ends_with('.') => short,
        _ => version,
    };
    format!("USB {version}")
}

/// The device class, or its interfaces' classes when it defers to them.
fn class_label(info: &UsbInfo) -> String {
    let name = |class| match (class, info.capture, info.playback) {
        (0x01, true, false) => "Audio (microphone)".to_owned(),
        (0x01, false, true) => "Audio (speakers)".to_owned(),
        (0x01, true, true) => "Audio (microphone and speakers)".to_owned(),
        _ => class_name(class),
    };
    if !matches!(info.class, 0x00 | 0xef) {
        return name(info.class);
    }
    let mut names: Vec<String> = info.interfaces.iter().map(|i| name(i.class)).collect();
    names.dedup();
    names.join(", ")
}

fn class_name(class: u8) -> String {
    match class {
        0x01 => "Audio",
        0x02 => "Communications",
        0x03 => "Human Interface Device",
        0x05 => "Physical",
        0x06 => "Imaging",
        0x07 => "Printer",
        0x08 => "Mass Storage",
        0x09 => "Hub",
        0x0a => "CDC Data",
        0x0b => "Smart Card",
        0x0d => "Content Security",
        0x0e => "Video",
        0x0f => "Personal Healthcare",
        0x10 => "Audio/Video",
        0x11 => "Billboard",
        0xdc => "Diagnostic",
        0xe0 => "Wireless Controller",
        0xef => "Miscellaneous",
        0xfe => "Application Specific",
        0xff => "Vendor Specific",
        other => return format!("Class {other:02x}"),
    }
    .to_owned()
}

/// An icon from the device class, or the first interface class that says
/// more (many devices declare "per interface").
fn icon_name(info: &UsbInfo) -> &'static str {
    let classes = std::iter::once((info.class, 0))
        .chain(info.interfaces.iter().map(|i| (i.class, i.protocol)));
    for (class, protocol) in classes {
        return match (class, protocol) {
            (0x01, _) => match (info.capture, info.playback) {
                (true, false) => "audio-input-microphone-symbolic",
                (false, true) => "audio-speakers-symbolic",
                _ => "audio-card-symbolic",
            },
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

    fn device(name: &str) -> UsbDevice {
        glib::Object::builder().property("name", name).build()
    }

    fn names(store: &gio::ListStore) -> Vec<String> {
        store
            .iter::<UsbDevice>()
            .map(|d| d.unwrap().name())
            .collect()
    }

    #[test]
    fn sync_only_touches_what_changed() {
        let store = gio::ListStore::new::<UsbDevice>();
        let (a, b, c, d) = (device("1-1"), device("1-2"), device("1-3"), device("1-4"));
        sync(&store, &[a.clone(), b.clone(), c.clone()]);
        assert_eq!(names(&store), ["1-1", "1-2", "1-3"]);

        let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = changes.clone();
        store.connect_items_changed(move |_, position, removed, added| {
            log.borrow_mut().push((position, removed, added));
        });
        // 1-2 unplugged, 1-4 plugged in: one removal, one insertion, 1-1 and 1-3 untouched.
        sync(&store, &[a.clone(), c.clone(), d.clone()]);
        assert_eq!(names(&store), ["1-1", "1-3", "1-4"]);
        assert_eq!(*changes.borrow(), [(1, 1, 0), (2, 0, 1)]);

        changes.borrow_mut().clear();
        sync(&store, &[a, c, d]);
        assert!(changes.borrow().is_empty());
        drop(b);
    }

    #[test]
    fn microphone_class_label() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-usb");
        let mic = &sysfs::scan(&root).unwrap()[4];
        assert_eq!(class_label(mic), "Audio (microphone)");
    }

    #[test]
    fn labels() {
        assert_eq!(speed_label("480"), "480 Mbit/s (High Speed)");
        assert_eq!(speed_label("2500"), "2500 Mbit/s");
        assert_eq!(version_label("2.10"), "USB 2.1");
        assert_eq!(version_label("2.00"), "USB 2.0");
        assert_eq!(version_label("2.01"), "USB 2.01");
        assert_eq!(version_label("3.20"), "USB 3.2");
    }

    #[test]
    fn icons_follow_device_then_interface_class() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sysfs-usb");
        let icons: Vec<_> = sysfs::scan(&root).unwrap().iter().map(icon_name).collect();
        assert_eq!(
            icons,
            [
                "media-removable-symbolic",        // hub
                "phone-symbolic",                  // MTP
                "drive-removable-media-symbolic",  // mass storage
                "input-mouse-symbolic",            // HID mouse
                "audio-input-microphone-symbolic", // capture only
            ]
        );
    }
}
