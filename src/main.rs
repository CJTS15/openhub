mod application;
mod bluez;
mod config;
mod model;
mod ui;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

use self::application::OpenhubApplication;

fn main() -> glib::ExitCode {
    gio::resources_register_include!("openhub.gresource").expect("Failed to register resources");

    glib::set_application_name("Openhub");

    OpenhubApplication::new().run()
}
