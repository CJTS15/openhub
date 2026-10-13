use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::Device)]
    pub struct Device {
        /// BlueZ object path, used by the service to send commands.
        #[property(get, set)]
        path: RefCell<String>,
        #[property(get, set)]
        address: RefCell<String>,
        /// Object path of the adapter the device belongs to.
        #[property(get, set)]
        adapter: RefCell<String>,
        #[property(get, set)]
        name: RefCell<String>,
        /// BlueZ icon name such as `audio-headset`; empty when unknown.
        #[property(get, set)]
        icon: RefCell<String>,
        #[property(get, set)]
        paired: Cell<bool>,
        #[property(get, set)]
        connected: Cell<bool>,
        #[property(get, set)]
        trusted: Cell<bool>,
        #[property(get, set)]
        blocked: Cell<bool>,
        /// Battery percentage, or -1 when the device doesn't report one.
        #[property(get, set, minimum = -1, maximum = 100, default = -1)]
        battery: Cell<i32>,
        /// A connect or disconnect is in progress.
        #[property(get, set)]
        busy: Cell<bool>,
    }

    impl Default for Device {
        fn default() -> Self {
            Self {
                path: RefCell::default(),
                address: RefCell::default(),
                adapter: RefCell::default(),
                name: RefCell::default(),
                icon: RefCell::default(),
                paired: Cell::default(),
                connected: Cell::default(),
                trusted: Cell::default(),
                blocked: Cell::default(),
                battery: Cell::new(-1),
                busy: Cell::default(),
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
