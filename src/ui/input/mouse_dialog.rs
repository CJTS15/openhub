use adw::prelude::*;

use crate::model::InputDevice;

/// A mouse or touchpad Openhub can't configure: what it is, and where its
/// pointer settings are.
pub fn new(device: &InputDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .build();
    let group = adw::PreferencesGroup::builder()
        .description(
            "Openhub can't change this device's settings. Pointer speed and scrolling \
             are in GNOME Settings, under Mouse & Touchpad.",
        )
        .build();
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
    let page = adw::PreferencesPage::new();
    page.add(&group);
    dialog.add(&page);
    dialog
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
