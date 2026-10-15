use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::PowerDevice)]
    pub struct PowerDevice {
        /// UPower object path.
        #[property(get, set)]
        path: RefCell<String>,
        #[property(get, set)]
        title: RefCell<String>,
        /// Level, state and connection, ready to show.
        #[property(get, set)]
        subtitle: RefCell<String>,
        #[property(get, set)]
        icon_name: RefCell<String>,
        /// Percentage, or -1 when there's none to show (a charger).
        #[property(get, set, minimum = -1.0, maximum = 100.0, default = -1.0)]
        level: Cell<f64>,
        /// Powers this computer (battery, charger, UPS) rather than a peripheral.
        #[property(get, set)]
        power_supply: Cell<bool>,
        /// UPower's device type.
        #[property(get, set)]
        kind: Cell<u32>,
        #[property(get, set)]
        serial: RefCell<String>,
        /// UPower got it from BlueZ rather than the kernel.
        #[property(get, set)]
        bluez: Cell<bool>,
        /// A laptop battery slot can be empty.
        #[property(get, set)]
        present: Cell<bool>,
    }

    impl Default for PowerDevice {
        fn default() -> Self {
            Self {
                path: RefCell::default(),
                title: RefCell::default(),
                subtitle: RefCell::default(),
                icon_name: RefCell::default(),
                level: Cell::new(-1.0),
                power_supply: Cell::default(),
                kind: Cell::default(),
                serial: RefCell::default(),
                bluez: Cell::default(),
                present: Cell::new(true),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerDevice {
        const NAME: &'static str = "OpenhubPowerDevice";
        type Type = super::PowerDevice;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PowerDevice {}
}

glib::wrapper! {
    /// A battery, charger or battery-powered peripheral, kept up to date by
    /// the UPower service.
    pub struct PowerDevice(ObjectSubclass<imp::PowerDevice>);
}
