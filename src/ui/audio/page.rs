use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::audio::{AudioManager, State};
use crate::model::{AudioDevice, PowerDevice};
use crate::upower::{PowerManager, convert};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct AudioPage {
        pub manager: AudioManager,
        pub stack: gtk::Stack,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioPage {
        const NAME: &'static str = "OpenhubAudioPage";
        type Type = super::AudioPage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for AudioPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let manager = &self.manager;

            let page = adw::PreferencesPage::new();
            page.add(&group("Output", "No output devices", &manager.outputs()));
            page.add(&group("Input", "No input devices", &manager.inputs()));
            self.stack.add_named(
                &gtk::Spinner::builder()
                    .spinning(true)
                    .halign(gtk::Align::Center)
                    .valign(gtk::Align::Center)
                    .width_request(32)
                    .height_request(32)
                    .build(),
                Some("loading"),
            );
            self.stack.add_named(
                &adw::StatusPage::builder()
                    .icon_name("audio-volume-muted-symbolic")
                    .title("Sound Unavailable")
                    .description("No PipeWire or PulseAudio server is running")
                    .build(),
                Some("unavailable"),
            );
            self.stack.add_named(&page, Some("ready"));
            self.stack
                .set_transition_type(gtk::StackTransitionType::Crossfade);
            obj.set_child(Some(&self.stack));

            manager
                .bind_property("state", &self.stack, "visible-child-name")
                .transform_to(|_, state: State| {
                    Some(match state {
                        State::Loading => "loading",
                        State::Unavailable => "unavailable",
                        State::Ready => "ready",
                    })
                })
                .sync_create()
                .build();

            // Bluetooth headphones' battery comes from UPower, by address.
            let update = glib::clone!(
                #[weak]
                obj,
                move || obj.update_batteries()
            );
            for store in [
                manager.outputs(),
                manager.inputs(),
                PowerManager::shared().peripherals(),
            ] {
                store.connect_items_changed(glib::clone!(
                    #[strong]
                    update,
                    move |_, _, _, _| update()
                ));
            }
        }
    }

    impl WidgetImpl for AudioPage {}
    impl BinImpl for AudioPage {}
}

glib::wrapper! {
    /// The Audio page: outputs and inputs, read-only for now.
    pub struct AudioPage(ObjectSubclass<imp::AudioPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AudioPage {
    fn update_batteries(&self) {
        let manager = &self.imp().manager;
        let batteries: Vec<PowerDevice> = PowerManager::shared()
            .peripherals()
            .iter()
            .flatten()
            .collect();
        for store in [manager.outputs(), manager.inputs()] {
            for device in store.iter::<AudioDevice>().flatten() {
                let mine: Vec<(String, String, f64)> = batteries
                    .iter()
                    .filter(|b| {
                        !device.address().is_empty()
                            && b.serial().eq_ignore_ascii_case(&device.address())
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
}

fn group(title: &str, empty: &str, devices: &gio::ListStore) -> adw::PreferencesGroup {
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
    list.bind_model(Some(devices), |item| {
        row(item.downcast_ref().unwrap()).upcast()
    });
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.add(&list);
    group
}

fn row(device: &AudioDevice) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    device
        .bind_property("title", &row, "title")
        .sync_create()
        .build();
    let property = |name| {
        gtk::PropertyExpression::new(AudioDevice::static_type(), gtk::Expression::NONE, name)
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
    // The default device carries a check mark, named for screen readers.
    let check = gtk::Image::from_icon_name("object-select-symbolic");
    check.set_tooltip_text(Some("Default"));
    check.update_property(&[gtk::accessible::Property::Label("Default")]);
    device
        .bind_property("is-default", &check, "visible")
        .sync_create()
        .build();
    row.add_suffix(&check);
    row
}
