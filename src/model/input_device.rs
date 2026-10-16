use std::cell::RefCell;

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::InputDevice)]
    pub struct InputDevice {
        /// Stable while connected; see `input::sysfs::InputInfo::key`.
        #[property(get, set)]
        key: RefCell<String>,
        #[property(get, set)]
        title: RefCell<String>,
        #[property(get, set)]
        icon_name: RefCell<String>,
        /// "USB", "Bluetooth" or "Built-in".
        #[property(get, set)]
        connection: RefCell<String>,
        /// Bluetooth address, matched against UPower's serial for battery.
        #[property(get, set)]
        uniq: RefCell<String>,
        /// USB device (sysfs name such as `3-9`) it hangs off, for battery.
        #[property(get, set)]
        usb_device: RefCell<String>,
        /// "85% Battery" when UPower reports one; set by the page.
        #[property(get, set)]
        battery: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InputDevice {
        const NAME: &'static str = "OpenhubInputDevice";
        type Type = super::InputDevice;
    }

    #[glib::derived_properties]
    impl ObjectImpl for InputDevice {}
}

glib::wrapper! {
    /// A mouse, touchpad or keyboard as the Mouse and Keyboard pages show it.
    pub struct InputDevice(ObjectSubclass<imp::InputDevice>);
}
