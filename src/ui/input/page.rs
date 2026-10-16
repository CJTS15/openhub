use std::cell::OnceCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::{keyboard_dialog, logitech_dialog, mouse_dialog, razer_dialog};
use crate::input::{InputManager, Kind};
use crate::model::{InputDevice, PowerDevice};
use crate::upower::PowerManager;
use crate::{hidpp, razer};

mod imp {
    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::InputPage)]
    pub struct InputPage {
        /// "mouse" or "keyboard", set in window.ui.
        #[property(get, set, construct_only)]
        pub kind: std::cell::RefCell<String>,
        pub manager: OnceCell<InputManager>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InputPage {
        const NAME: &'static str = "OpenhubInputPage";
        type Type = super::InputPage;
        type ParentType = adw::Bin;
    }

    #[glib::derived_properties]
    impl ObjectImpl for InputPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let (kinds, title, empty): (&'static [Kind], _, _) = match self.kind.borrow().as_str() {
                "keyboard" => (&[Kind::Keyboard], "Keyboards", "No keyboard connected"),
                _ => (
                    &[Kind::Mouse, Kind::Touchpad, Kind::PointingStick],
                    "Mice and Touchpads",
                    "No mouse or touchpad connected",
                ),
            };
            let manager = self.manager.get_or_init(|| InputManager::new(kinds));
            let mice = !kinds.contains(&Kind::Keyboard);

            let list = gtk::ListBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .css_classes(["boxed-list"])
                .build();
            list.set_placeholder(Some(
                &gtk::Label::builder()
                    .label(empty)
                    .margin_top(18)
                    .margin_bottom(18)
                    .css_classes(["dim-label"])
                    .build(),
            ));
            list.bind_model(Some(&manager.devices()), move |item| {
                row(item.downcast_ref().unwrap(), mice).upcast()
            });
            let group = adw::PreferencesGroup::builder().title(title).build();
            group.add(&list);
            let page = adw::PreferencesPage::new();
            page.add(&group);
            obj.set_child(Some(&page));

            PowerManager::shared()
                .peripherals()
                .connect_items_changed(glib::clone!(
                    #[weak]
                    obj,
                    move |_, _, _, _| obj.update_batteries()
                ));
            crate::ui::rescan_while_mapped(
                &*obj,
                glib::clone!(
                    #[weak]
                    obj,
                    move || {
                        obj.imp().manager.get().unwrap().refresh();
                        obj.update_batteries();
                    }
                ),
            );
        }
    }

    impl WidgetImpl for InputPage {}
    impl BinImpl for InputPage {}
}

glib::wrapper! {
    /// The Mouse or Keyboard page: one list of devices, read-only for now.
    pub struct InputPage(ObjectSubclass<imp::InputPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InputPage {
    /// Battery from UPower: a Bluetooth device by its address, a wireless
    /// one by the USB receiver it hangs off.
    fn update_batteries(&self) {
        let Some(manager) = self.imp().manager.get() else {
            return;
        };
        let batteries: Vec<PowerDevice> = PowerManager::shared()
            .peripherals()
            .iter()
            .flatten()
            .collect();
        for device in manager.devices().iter::<InputDevice>().flatten() {
            let mine: Vec<(String, f64)> = batteries
                .iter()
                .filter(|b| {
                    let by_address = !device.uniq().is_empty()
                        && b.serial().eq_ignore_ascii_case(&device.uniq());
                    let by_receiver =
                        !device.usb_device().is_empty() && b.usb_device() == device.usb_device();
                    by_address || by_receiver
                })
                .map(|b| (b.level_text(), b.fraction()))
                .collect();
            let text = battery_text(&mine);
            if device.battery() != text {
                device.set_battery(text);
            }
        }
    }
}

/// `mice` is true on the Mouse page.
fn row(device: &InputDevice, mice: bool) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    device
        .bind_property("title", &row, "title")
        .sync_create()
        .build();
    let property = |name| {
        gtk::PropertyExpression::new(InputDevice::static_type(), gtk::Expression::NONE, name)
    };
    gtk::ClosureExpression::with_callback(
        [property("connection"), property("battery")],
        |values| {
            let connection: String = values[1].get().unwrap_or_default();
            let battery: String = values[2].get().unwrap_or_default();
            subtitle(&connection, &battery)
        },
    )
    .bind(&row, "subtitle", Some(device));
    let icon = gtk::Image::new();
    device
        .bind_property("icon-name", &icon, "icon-name")
        .sync_create()
        .build();
    row.add_prefix(&icon);
    // Logitech and Razer mice open their settings, other mice and touchpads a
    // dialog that says where theirs are, keyboards their VIA settings.
    let uniq = device.uniq();
    let usb_device = device.usb_device();
    let logitech = hidpp::find()
        .into_iter()
        .find(|node| !uniq.is_empty() && node.uniq.eq_ignore_ascii_case(&uniq));
    let razer = razer::find()
        .into_iter()
        .find(|node| !usb_device.is_empty() && node.usb_device == usb_device);

    row.set_activatable(true);
    row.connect_activated(glib::clone!(
        #[weak]
        device,
        move |row| {
            let dialog = match (&logitech, &razer) {
                (Some(node), _) => logitech_dialog::new(&device, node.clone()),
                (None, Some(node)) => razer_dialog::new(&device, node.clone()),
                (None, None) if mice => mouse_dialog::new(&device),
                (None, None) => keyboard_dialog::new(&device),
            };
            dialog.present(Some(row));
        }
    ));
    row
}

/// "85% Battery" for the lowest of a device's batteries (a receiver can
/// report more than one), or nothing. Each entry is (level text, fraction).
fn battery_text(batteries: &[(String, f64)]) -> String {
    batteries
        .iter()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map_or_else(String::new, |(level, _)| format!("{level} Battery"))
}

/// "Connected · Bluetooth · 85% Battery"; a built-in device is just "Built-in".
fn subtitle(connection: &str, battery: &str) -> String {
    let connected = if connection == "Built-in" {
        ""
    } else {
        "Connected"
    };
    [connected, connection, battery]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles() {
        let battery = battery_text(&[("85%".into(), 0.85)]);
        assert_eq!(
            subtitle("Bluetooth", &battery),
            "Connected · Bluetooth · 85% Battery"
        );
        assert_eq!(subtitle("USB", ""), "Connected · USB");
        assert_eq!(subtitle("Built-in", ""), "Built-in");
        assert_eq!(
            battery_text(&[("64%".into(), 0.64), ("High".into(), 0.75)]),
            "64% Battery"
        );
        assert_eq!(battery_text(&[]), "");
    }
}
