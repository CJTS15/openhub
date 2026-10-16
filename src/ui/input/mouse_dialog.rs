use adw::prelude::*;
use gtk::glib;

use super::ratbag_dialog;
use crate::model::InputDevice;
use crate::ratbag;

/// A mouse or touchpad that isn't Logitech or Razer. If ratbagd knows it
/// (by USB ID), its settings from there; otherwise what it is, and where
/// its pointer settings are.
pub fn new(device: &InputDevice) -> adw::PreferencesDialog {
    let Some((vendor, product)) = usb_ids(&device.usb_device()) else {
        let dialog = adw::PreferencesDialog::builder()
            .title(device.title())
            .build();
        let page = adw::PreferencesPage::new();
        page.add(&info(device, false));
        dialog.add(&page);
        return dialog;
    };
    let (dialog, page, loading) = loading(device);
    glib::spawn_future_local(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        page,
        #[weak]
        device,
        async move {
            let found = match zbus::Connection::system().await {
                Ok(conn) => ratbag::find(&conn, vendor, product)
                    .await
                    .map(|mouse| mouse.map(|mouse| (conn, mouse))),
                Err(error) => Err(error),
            };
            page.remove(&loading);
            match found {
                Ok(Some((conn, mouse))) => ratbag_dialog::fill(&dialog, &page, conn, mouse),
                Ok(None) => page.add(&info(&device, false)),
                // Usually: ratbagd isn't installed.
                Err(_) => page.add(&info(&device, true)),
            }
        }
    ));
    dialog
}

/// Connection, battery, and why there's nothing to change.
fn info(device: &InputDevice, without_ratbagd: bool) -> adw::PreferencesGroup {
    let mut text = "Openhub can't change this device's settings. Pointer speed and scrolling \
                    are in GNOME Settings, under Mouse & Touchpad."
        .to_owned();
    if without_ratbagd {
        text.push_str(
            " If libratbag supports this mouse, installing ratbagd (Piper's backend) lets \
             Openhub change its DPI and buttons.",
        );
    }
    let group = adw::PreferencesGroup::builder().description(&text).build();
    for (label, value) in [
        ("Connection", device.connection()),
        ("Battery", device.battery()),
    ] {
        if !value.is_empty() {
            group.add(
                &adw::ActionRow::builder()
                    .title(label)
                    .subtitle(&value)
                    .use_markup(false)
                    .css_classes(["property"])
                    .build(),
            );
        }
    }
    group
}

/// The USB vendor and product ID of a device such as `3-1.3`.
fn usb_ids(usb_device: &str) -> Option<(u16, u16)> {
    if usb_device.is_empty() {
        return None;
    }
    let dir = std::path::Path::new("/sys/bus/usb/devices").join(usb_device);
    let id = |file: &str| {
        let text = std::fs::read_to_string(dir.join(file)).ok()?;
        u16::from_str_radix(text.trim(), 16).ok()
    };
    Some((id("idVendor")?, id("idProduct")?))
}

/// The frame the Logitech and Razer dialogs fill once the mouse is read:
/// the dialog, its page, and the spinner group to remove.
pub(super) fn loading(
    device: &InputDevice,
) -> (
    adw::PreferencesDialog,
    adw::PreferencesPage,
    adw::PreferencesGroup,
) {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        // Sized up front: it opens with only a spinner while the mouse is read.
        .content_width(560)
        .content_height(480)
        .build();
    let page = adw::PreferencesPage::new();
    let spinner = adw::PreferencesGroup::new();
    spinner.add(
        &gtk::Spinner::builder()
            .spinning(true)
            .height_request(32)
            .margin_top(24)
            .margin_bottom(24)
            .build(),
    );
    page.add(&spinner);
    dialog.add(&page);
    (dialog, page, spinner)
}

pub(super) fn message(text: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.add(
        &adw::StatusPage::builder()
            .icon_name("input-mouse-symbolic")
            .title("Can't Read This Mouse")
            .description(text)
            .css_classes(["compact"])
            .build(),
    );
    group
}
