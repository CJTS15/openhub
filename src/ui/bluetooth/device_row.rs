use adw::prelude::*;
use gtk::glib;

use crate::model::Device;

pub fn new(device: &Device) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    let icon = gtk::Image::new();
    row.add_prefix(&icon);

    let update = glib::clone!(
        #[weak]
        row,
        #[weak]
        icon,
        move |device: &Device| {
            row.set_title(&device.name());
            row.set_subtitle(&status(device));
            icon.set_icon_name(Some(&icon_name(&device.icon())));
        }
    );
    update(device);
    device.connect_notify_local(None, move |device, _| update(device));
    row
}

fn status(device: &Device) -> String {
    match (device.connected(), device.battery()) {
        (false, _) => "Not connected".into(),
        (true, -1) => "Connected".into(),
        (true, battery) => format!("Connected · {battery}% battery"),
    }
}

/// BlueZ gives a freedesktop icon name like `audio-headset`; use its symbolic
/// variant when the theme has one.
fn icon_name(bluez_icon: &str) -> String {
    let symbolic = format!("{bluez_icon}-symbolic");
    let has_icon = gtk::gdk::Display::default()
        .is_some_and(|display| gtk::IconTheme::for_display(&display).has_icon(&symbolic));
    if !bluez_icon.is_empty() && has_icon {
        symbolic
    } else {
        "bluetooth-symbolic".into()
    }
}
