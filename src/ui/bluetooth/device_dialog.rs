use adw::prelude::*;
use gtk::glib;

use crate::bluez::BluetoothManager;
use crate::model::Device;

/// Details for a paired device: address, battery, trust and block, Forget.
// ponytail: stays open if the device is forgotten elsewhere; close it on removal if that bites.
pub fn new(manager: &BluetoothManager, device: &Device) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::new();
    device
        .bind_property("name", &dialog, "title")
        .sync_create()
        .build();

    let info = adw::PreferencesGroup::new();
    let address = adw::ActionRow::builder()
        .title("Address")
        .subtitle(device.address())
        .subtitle_selectable(true)
        .css_classes(["property"])
        .build();
    let battery = adw::ActionRow::builder()
        .title("Battery")
        .css_classes(["property"])
        .build();
    device
        .bind_property("battery", &battery, "subtitle")
        .transform_to(|_, battery: i32| Some(format!("{battery}%")))
        .sync_create()
        .build();
    device
        .bind_property("battery", &battery, "visible")
        .transform_to(|_, battery: i32| Some(battery >= 0))
        .sync_create()
        .build();
    info.add(&address);
    info.add(&battery);

    let access = adw::PreferencesGroup::new();
    access.add(&switch_row(
        &dialog,
        manager,
        device,
        "trusted",
        "Trusted",
        "Can connect without asking",
    ));
    access.add(&switch_row(
        &dialog,
        manager,
        device,
        "blocked",
        "Blocked",
        "Refuses all connections from this device",
    ));

    let forget = gtk::Button::builder()
        .label("Forget Device")
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    forget.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        manager,
        #[weak]
        device,
        move |_| confirm_forget(&dialog, &manager, &device)
    ));
    let danger = adw::PreferencesGroup::new();
    danger.add(&forget);

    let page = adw::PreferencesPage::new();
    page.add(&info);
    page.add(&access);
    page.add(&danger);
    dialog.add(&page);
    dialog
}

/// A switch for a boolean device property that BlueZ lets us set.
fn switch_row(
    dialog: &adw::PreferencesDialog,
    manager: &BluetoothManager,
    device: &Device,
    property: &'static str,
    title: &str,
    subtitle: &str,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    device
        .bind_property(property, &row, "active")
        .sync_create()
        .build();
    row.connect_active_notify(glib::clone!(
        #[weak]
        dialog,
        #[strong]
        manager,
        #[weak]
        device,
        move |row| {
            let value = row.is_active();
            // Equal when the change came from BlueZ through the binding above.
            if value == device.property::<bool>(property) {
                return;
            }
            glib::spawn_future_local(glib::clone!(
                #[weak]
                dialog,
                #[strong]
                manager,
                #[weak]
                device,
                #[weak]
                row,
                async move {
                    let result = match property {
                        "trusted" => manager.set_trusted(&device, value).await,
                        _ => manager.set_blocked(&device, value).await,
                    };
                    if let Err(err) = result {
                        glib::g_warning!("openhub", "Can't set {property}={value}: {err}");
                        toast(&dialog, &format!("Couldn't change {}", row.title()));
                        row.set_active(device.property::<bool>(property));
                    }
                }
            ));
        }
    ));
    row
}

fn confirm_forget(dialog: &adw::PreferencesDialog, manager: &BluetoothManager, device: &Device) {
    let confirm = adw::AlertDialog::new(
        Some(&format!("Forget {}?", device.name())),
        Some("You'll need to pair it again to use it."),
    );
    confirm.add_response("cancel", "_Cancel");
    confirm.add_response("forget", "_Forget");
    confirm.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
    confirm.set_close_response("cancel");
    confirm.connect_response(
        Some("forget"),
        glib::clone!(
            #[weak]
            dialog,
            #[strong]
            manager,
            #[weak]
            device,
            move |_, _| {
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    dialog,
                    #[strong]
                    manager,
                    #[weak]
                    device,
                    async move {
                        match manager.forget_device(&device).await {
                            Ok(()) => {
                                dialog.close();
                            }
                            Err(err) => {
                                glib::g_warning!(
                                    "openhub",
                                    "Can't forget {}: {err}",
                                    device.path()
                                );
                                toast(&dialog, &format!("Couldn't forget {}", device.name()));
                            }
                        }
                    }
                ));
            }
        ),
    );
    confirm.present(Some(dialog));
}

fn toast(dialog: &adw::PreferencesDialog, title: &str) {
    dialog.add_toast(adw::Toast::builder().title(title).use_markup(false).build());
}
