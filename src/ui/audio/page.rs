use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use super::device_dialog;
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
            page.add(&group(
                manager,
                "Output",
                "No output devices",
                &manager.outputs(),
            ));
            page.add(&group(
                manager,
                "Input",
                "No input devices",
                &manager.inputs(),
            ));
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
            // Level meters run only while the page is on screen.
            obj.connect_map(|page| page.imp().manager.set_metering(true));
            obj.connect_unmap(|page| page.imp().manager.set_metering(false));

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

fn group(
    manager: &AudioManager,
    title: &str,
    empty: &str,
    devices: &gio::ListStore,
) -> adw::PreferencesGroup {
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
    list.bind_model(
        Some(devices),
        glib::clone!(
            #[strong]
            manager,
            move |item| row(&manager, item.downcast_ref().unwrap()).upcast()
        ),
    );
    let group = adw::PreferencesGroup::builder().title(title).build();
    group.add(&list);
    group
}

fn row(manager: &AudioManager, device: &AudioDevice) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    device
        .bind_property("title", &row, "title")
        .sync_create()
        .build();
    let property = |name| {
        gtk::PropertyExpression::new(AudioDevice::static_type(), gtk::Expression::NONE, name)
    };
    gtk::ClosureExpression::with_callback(
        [
            property("connection"),
            property("codec"),
            property("battery"),
        ],
        |values| {
            let parts: Vec<String> = values[1..]
                .iter()
                .map(|value| value.get::<String>().unwrap_or_default())
                .filter(|part| !part.is_empty())
                .collect();
            parts.join(" · ")
        },
    )
    .bind(&row, "subtitle", Some(device));

    let icon = gtk::Image::new();
    device
        .bind_property("icon-name", &icon, "icon-name")
        .sync_create()
        .build();
    row.add_prefix(&icon);
    // Clicking a row opens its details and equalizer; the default carries
    // a check mark and is set from there.
    row.set_activatable(true);
    row.connect_activated(glib::clone!(
        #[strong]
        manager,
        #[weak]
        device,
        move |row| device_dialog::new(&manager, &device).present(Some(row))
    ));

    // Inputs show their live level here; outputs keep the same width
    // empty, so every row's mute button and slider line up.
    if device.output() {
        row.add_suffix(&gtk::Box::builder().width_request(METER_WIDTH).build());
    } else {
        let meter = gtk::LevelBar::builder()
            .width_request(METER_WIDTH)
            .valign(gtk::Align::Center)
            .build();
        for name in ["low", "high", "full"] {
            meter.remove_offset_value(Some(name));
        }
        meter.update_property(&[gtk::accessible::Property::Label("Input level")]);
        device
            .bind_property("peak", &meter, "value")
            .sync_create()
            .build();
        row.add_suffix(&meter);
    }

    let mute = gtk::Button::builder()
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let output = device.output();
    device
        .bind_property("muted", &mute, "icon-name")
        .transform_to(move |_, muted: bool| {
            Some(match (output, muted) {
                (true, true) => "audio-volume-muted-symbolic",
                (true, false) => "audio-volume-high-symbolic",
                (false, true) => "microphone-sensitivity-muted-symbolic",
                (false, false) => "microphone-sensitivity-high-symbolic",
            })
        })
        .sync_create()
        .build();
    device
        .bind_property("muted", &mute, "tooltip-text")
        .transform_to(|_, muted: bool| Some(if muted { "Unmute" } else { "Mute" }))
        .sync_create()
        .build();
    mute.connect_clicked(glib::clone!(
        #[strong]
        manager,
        #[weak]
        device,
        move |_| manager.set_muted(&device, !device.muted())
    ));
    row.add_suffix(&mute);

    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
    volume.set_width_request(160);
    volume.set_valign(gtk::Align::Center);
    volume.update_property(&[gtk::accessible::Property::Label("Volume")]);
    device
        .bind_property("volume", &volume.adjustment(), "value")
        .sync_create()
        .build();
    // Only the user's moves reach the server; updates from it come back
    // through the binding without echoing.
    volume.connect_change_value(glib::clone!(
        #[strong]
        manager,
        #[weak]
        device,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, _, value| {
            manager.set_volume(&device, value);
            glib::Propagation::Proceed
        }
    ));
    row.add_suffix(&volume);
    // A fixed slot keeps its space when the check mark is hidden, so rows
    // don't shift as the default changes.
    let check = gtk::Image::from_icon_name("object-select-symbolic");
    check.set_tooltip_text(Some("Default"));
    check.update_property(&[gtk::accessible::Property::Label("Default")]);
    device
        .bind_property("is-default", &check, "visible")
        .sync_create()
        .build();
    let slot = gtk::Box::builder()
        .width_request(24)
        .halign(gtk::Align::End)
        .build();
    slot.append(&check);
    row.add_suffix(&slot);
    row
}

/// The input level meter is short, as in GNOME Settings.
const METER_WIDTH: i32 = 40;
