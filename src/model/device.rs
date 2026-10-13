use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Device)]
    pub struct Device {
        #[property(get, set)]
        name: RefCell<String>,
        /// BlueZ icon name such as `audio-headset`; empty when unknown.
        #[property(get, set)]
        icon: RefCell<String>,
        #[property(get, set)]
        paired: Cell<bool>,
        #[property(get, set)]
        connected: Cell<bool>,
        /// Battery percentage, or -1 when the device doesn't report one.
        #[property(get, set, minimum = -1, maximum = 100, default = -1)]
        battery: Cell<i32>,
    }

    impl Default for Device {
        fn default() -> Self {
            Self {
                name: RefCell::default(),
                icon: RefCell::default(),
                paired: Cell::default(),
                connected: Cell::default(),
                battery: Cell::new(-1),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Device {
        const NAME: &'static str = "OpenhubDevice";
        type Type = super::Device;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Device {}
}

glib::wrapper! {
    /// A Bluetooth device as the UI sees it. Kept up to date by the bluez service.
    pub struct Device(ObjectSubclass<imp::Device>);
}

impl Default for Device {
    fn default() -> Self {
        glib::Object::new()
    }
}
