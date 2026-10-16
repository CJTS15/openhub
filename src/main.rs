mod application;
mod audio;
mod bluez;
mod config;
mod eq;
mod hidpp;
mod input;
mod model;
mod razer;
mod ui;
mod upower;
mod usb;
mod watch;
mod window;

use gtk::prelude::*;
use gtk::{gio, glib};

use self::application::OpenhubApplication;

fn main() -> glib::ExitCode {
    // `openhub --apply-mice hidrawN`, run by systemd when a Logitech mouse
    // connects: restores its saved settings and exits, without a window.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, node] = args.as_slice()
        && flag == "--apply-mice"
    {
        return match hidpp::apply(node) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Couldn't restore {node}'s settings: {error}");
                glib::ExitCode::FAILURE
            }
        };
    }

    gio::resources_register_include!("openhub.gresource").expect("Failed to register resources");

    glib::set_application_name("Openhub");

    OpenhubApplication::new().run()
}
