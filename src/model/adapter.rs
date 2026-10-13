use std::cell::{Cell, RefCell};

use gtk::glib;
use gtk::glib::prelude::*;
use gtk::glib::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::Adapter)]
    pub struct Adapter {
        /// BlueZ object path, used by the service to send commands.
        #[property(get, set)]
        path: RefCell<String>,
        #[property(get, set)]
        name: RefCell<String>,
        #[property(get, set)]
        powered: Cell<bool>,
        #[property(get, set)]
        discovering: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Adapter {
        const NAME: &'static str = "OpenhubAdapter";
        type Type = super::Adapter;
    }

    #[glib::derived_properties]
    impl ObjectImpl for Adapter {}
}

glib::wrapper! {
    /// A local Bluetooth adapter. Kept up to date by the bluez service.
    pub struct Adapter(ObjectSubclass<imp::Adapter>);
}

impl Default for Adapter {
    fn default() -> Self {
        glib::Object::new()
    }
}
