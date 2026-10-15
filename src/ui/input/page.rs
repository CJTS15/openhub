use std::cell::OnceCell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::input::{InputManager, Kind};
use crate::model::{InputDevice, PowerDevice};
use crate::upower::{PowerManager, convert};

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
            list.bind_model(Some(&manager.devices()), |item| {
                row(item.downcast_ref().unwrap()).upcast()
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
            let mine: Vec<(String, String, f64)> = batteries
                .iter()
                .filter(|b| {
                    let by_address = !device.uniq().is_empty()
                        && b.serial().eq_ignore_ascii_case(&device.uniq());
                    let by_receiver =
                        !device.usb_device().is_empty() && b.usb_device() == device.usb_device();
                    by_address || by_receiver
                })
                .map(|b| (b.title(), b.level_text(), b.fraction()))
                .collect();
            let (text, _) = convert::usb_battery_summary(&mine);
            if device.battery() != text {
                device.set_battery(text);
            }
        }
    }
}

fn row(device: &InputDevice) -> adw::ActionRow {
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
            [connection, battery]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ")
        },
    )
    .bind(&row, "subtitle", Some(device));
    let icon = gtk::Image::new();
    device
        .bind_property("icon-name", &icon, "icon-name")
        .sync_create()
        .build();
    row.add_prefix(&icon);
    row
}
