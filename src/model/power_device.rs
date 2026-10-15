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
        /// "85 %", or a coarse word such as "High" for devices that only report that.
        #[property(get, set)]
        level_text: RefCell<String>,
        /// Where its bar sits, 0 to 1.
        #[property(get, set)]
        fraction: Cell<f64>,
        /// UPower `WarningLevel`; see `convert::warning`.
        #[property(get, set)]
        warning_level: Cell<u32>,
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
        /// "Bluetooth", "USB" or "Other", for peripherals.
        #[property(get, set)]
        connection: RefCell<String>,
        /// The USB device (sysfs name such as `3-2`) whose receiver reports
        /// this battery; empty when it isn't behind one.
        #[property(get, set)]
        usb_device: RefCell<String>,

        // Details, as UPower reports them; 0 or empty when unknown.
        #[property(get, set)]
        vendor: RefCell<String>,
        #[property(get, set)]
        model: RefCell<String>,
        #[property(get, set)]
        technology: RefCell<String>,
        /// Full charge capacity against design capacity, in percent.
        #[property(get, set)]
        capacity: Cell<f64>,
        /// Watt-hours.
        #[property(get, set)]
        energy: Cell<f64>,
        #[property(get, set)]
        energy_full: Cell<f64>,
        #[property(get, set)]
        energy_full_design: Cell<f64>,
        /// Watts.
        #[property(get, set)]
        energy_rate: Cell<f64>,
        /// Volts.
        #[property(get, set)]
        voltage: Cell<f64>,
        /// -1 when unknown.
        #[property(get, set, default = -1)]
        charge_cycles: Cell<i32>,

        // Charge limit ("Preserve battery health").
        #[property(get, set)]
        charge_limit_supported: Cell<bool>,
        #[property(get, set)]
        charge_limit_enabled: Cell<bool>,
        /// Percent where charging stops while the limit is on.
        #[property(get, set)]
        charge_end_threshold: Cell<u32>,
    }

    impl Default for PowerDevice {
        fn default() -> Self {
            Self {
                path: RefCell::default(),
                title: RefCell::default(),
                subtitle: RefCell::default(),
                icon_name: RefCell::default(),
                level: Cell::new(-1.0),
                level_text: RefCell::default(),
                fraction: Cell::default(),
                warning_level: Cell::default(),
                power_supply: Cell::default(),
                kind: Cell::default(),
                serial: RefCell::default(),
                bluez: Cell::default(),
                present: Cell::new(true),
                connection: RefCell::default(),
                usb_device: RefCell::default(),
                vendor: RefCell::default(),
                model: RefCell::default(),
                technology: RefCell::default(),
                capacity: Cell::default(),
                energy: Cell::default(),
                energy_full: Cell::default(),
                energy_full_design: Cell::default(),
                energy_rate: Cell::default(),
                voltage: Cell::default(),
                charge_cycles: Cell::new(-1),
                charge_limit_supported: Cell::default(),
                charge_limit_enabled: Cell::default(),
                charge_end_threshold: Cell::default(),
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
