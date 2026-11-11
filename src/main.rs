mod application;
mod audio;
mod battery;
mod bluez;
mod config;
mod ddc;
mod display;
mod eq;
mod fwupd;
mod hidpp;
mod i18n;
mod input;
mod model;
mod ratbag;
mod razer;
mod shell_extension;
mod sysinfo;
mod ui;
mod upower;
mod usb;
mod via;
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

    // Before any UI: GtkBuilder translates .ui files with the default domain.
    // SAFETY: setlocale isn't thread-safe; this runs before GTK, the sound
    // server or any other thread starts.
    unsafe { gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "") };
    let _ = gettextrs::bindtextdomain(config::GETTEXT_PACKAGE, config::LOCALEDIR);
    let _ = gettextrs::bind_textdomain_codeset(config::GETTEXT_PACKAGE, "UTF-8");
    let _ = gettextrs::textdomain(config::GETTEXT_PACKAGE);

    gio::resources_register_include!("openhub.gresource").expect("Failed to register resources");

    glib::set_application_name("Openhub");

    OpenhubApplication::new().run()
}
