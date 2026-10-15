//! Keeps a list of input devices of some kinds in step with sysfs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::sysfs::{self, InputInfo, Kind};
use crate::model::InputDevice;
use crate::upower::convert::usb_owner;

#[derive(Debug)]
pub struct InputManager {
    kinds: &'static [Kind],
    devices: gio::ListStore,
    /// The objects from the last scan, by key, so rows survive rescans.
    known: RefCell<HashMap<String, InputDevice>>,
}

impl InputManager {
    pub fn new(kinds: &'static [Kind]) -> Self {
        Self {
            kinds,
            devices: gio::ListStore::new::<InputDevice>(),
            known: RefCell::default(),
        }
    }

    /// `InputDevice` items of this manager's kinds, sorted by name.
    pub fn devices(&self) -> gio::ListStore {
        self.devices.clone()
    }

    /// Rescans sysfs; the list changes only when devices come or go.
    pub fn refresh(&self) {
        let root = Path::new(sysfs::ROOT);
        let infos = sysfs::scan(root).unwrap_or_else(|err| {
            glib::g_warning!("openhub", "Can't read {}: {err}", sysfs::ROOT);
            Vec::new()
        });
        let mut known = self.known.borrow_mut();
        let mut next = HashMap::new();
        let mut wanted = Vec::new();
        for info in infos.iter().filter(|info| self.kinds.contains(&info.kind)) {
            let device = known
                .remove(&info.key)
                .unwrap_or_else(|| new_device(root, info));
            next.insert(info.key.clone(), device.clone());
            wanted.push(device);
        }
        *known = next;
        let current: Vec<InputDevice> = self.devices.iter().flatten().collect();
        if current != wanted {
            self.devices.splice(0, self.devices.n_items(), &wanted);
        }
    }
}

fn new_device(root: &Path, info: &InputInfo) -> InputDevice {
    let icon = match info.kind {
        Kind::Keyboard => "input-keyboard-symbolic",
        Kind::Touchpad => "input-touchpad-symbolic",
        Kind::Mouse | Kind::PointingStick => "input-mouse-symbolic",
    };
    // Its sysfs path runs through the USB interface it hangs off, if any.
    let usb_device = root
        .join(&info.node)
        .canonicalize()
        .ok()
        .and_then(|path| usb_owner(&path))
        .unwrap_or_default();
    glib::Object::builder()
        .property("key", &info.key)
        .property("title", &info.name)
        .property("icon-name", icon)
        .property("connection", info.connection)
        .property("uniq", &info.uniq)
        .property("usb-device", usb_device)
        .build()
}
