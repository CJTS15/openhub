use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::AudioDevice)]
    pub struct AudioDevice {
        /// PulseAudio name, such as `alsa_output.pci-…HDMI1__sink`.
        #[property(get, set)]
        name: RefCell<String>,
        #[property(get, set)]
        title: RefCell<String>,
        #[property(get, set)]
        icon_name: RefCell<String>,
        /// "Bluetooth", "USB", "Built-in" or "Virtual".
        #[property(get, set)]
        connection: RefCell<String>,
        /// Bluetooth address, matched against UPower's serial for battery.
        #[property(get, set)]
        address: RefCell<String>,
        #[property(get, set)]
        is_default: Cell<bool>,
        /// "85 % battery" when UPower reports one; set by the page.
        #[property(get, set)]
        battery: RefCell<String>,
        /// Output (sink) rather than input (source).
        #[property(get, set)]
        output: Cell<bool>,
        /// Volume as a fraction of 100 %.
        #[property(get, set)]
        volume: Cell<f64>,
        #[property(get, set)]
        muted: Cell<bool>,
        /// Channel count, needed to set the volume back.
        #[property(get, set)]
        channels: Cell<u32>,
        /// Bluetooth codec such as "LDAC"; empty for other devices.
        #[property(get, set)]
        codec: RefCell<String>,
        /// Input level, 0 to 1, while the Audio page is showing.
        #[property(get, set)]
        peak: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioDevice {
        const NAME: &'static str = "OpenhubAudioDevice";
        type Type = super::AudioDevice;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioDevice {}
}

glib::wrapper! {
    /// An audio output or input, kept up to date by the audio service.
    pub struct AudioDevice(ObjectSubclass<imp::AudioDevice>);
}
