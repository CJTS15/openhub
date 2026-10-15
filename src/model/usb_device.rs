use std::cell::{Cell, RefCell};

use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;
use gtk::{gio, glib};

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::UsbDevice)]
    pub struct UsbDevice {
        /// Sysfs name such as `3-1.2`.
        #[property(get, set)]
        name: RefCell<String>,
        #[property(get, set)]
        title: RefCell<String>,
        #[property(get, set)]
        subtitle: RefCell<String>,
        #[property(get, set)]
        icon_name: RefCell<String>,
        /// Built into the computer, such as a webcam or fingerprint reader.
        #[property(get, set)]
        internal: Cell<bool>,
        #[property(get, set)]
        hub: Cell<bool>,
        /// `vendor:product` in hex, such as `05e3:0610`.
        #[property(get, set)]
        id: RefCell<String>,
        /// The rest are for the details dialog, ready to show; empty when unknown.
        #[property(get, set)]
        serial: RefCell<String>,
        #[property(get, set)]
        speed: RefCell<String>,
        #[property(get, set)]
        version: RefCell<String>,
        #[property(get, set)]
        class_name: RefCell<String>,
        #[property(get, set)]
        driver: RefCell<String>,
        #[property(get, set)]
        max_power: RefCell<String>,
        /// Disks (`sdb`…) when it's a storage device; empty otherwise.
        #[property(get, set)]
        disks: RefCell<Vec<String>>,
        /// "80 % battery" when it's a receiver whose devices report a battery
        /// (the lowest of them); empty otherwise. Set by the USB page from UPower.
        #[property(get, set)]
        battery: RefCell<String>,
        /// One line per battery behind it, for the details dialog.
        #[property(get, set)]
        batteries: RefCell<Vec<String>>,
        /// `UsbDevice` items plugged into this one, when it's a hub.
        pub(super) children: gio::ListStore,
    }

    impl Default for UsbDevice {
        fn default() -> Self {
            Self {
                name: RefCell::default(),
                title: RefCell::default(),
                subtitle: RefCell::default(),
                icon_name: RefCell::default(),
                internal: Cell::default(),
                hub: Cell::default(),
                id: RefCell::default(),
                serial: RefCell::default(),
                speed: RefCell::default(),
                version: RefCell::default(),
                class_name: RefCell::default(),
                driver: RefCell::default(),
                max_power: RefCell::default(),
                disks: RefCell::default(),
                battery: RefCell::default(),
                batteries: RefCell::default(),
                children: gio::ListStore::new::<super::UsbDevice>(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for UsbDevice {
        const NAME: &'static str = "OpenhubUsbDevice";
        type Type = super::UsbDevice;
    }

    #[glib::derived_properties]
    impl ObjectImpl for UsbDevice {}
}

glib::wrapper! {
    /// A USB device as the UI sees it. Built by the USB service.
    pub struct UsbDevice(ObjectSubclass<imp::UsbDevice>);
}

impl UsbDevice {
    /// `UsbDevice` items plugged into this one, when it's a hub.
    pub fn children(&self) -> gio::ListStore {
        self.imp().children.clone()
    }
}
