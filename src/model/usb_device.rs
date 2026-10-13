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
