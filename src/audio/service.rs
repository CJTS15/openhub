//! Lists audio outputs and inputs through the PulseAudio protocol, which
//! PipeWire serves as `pipewire-pulse`. libpulse runs on GLib's default main
//! context, so its callbacks arrive on the GTK main thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use libpulse_binding as pulse;
use libpulse_glib_binding as pulse_glib;
use pulse::callbacks::ListResult;
use pulse::context::subscribe::InterestMaskSet;
use pulse::context::{Context, FlagSet};
use pulse::proplist::Proplist;

use crate::model::AudioDevice;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "OpenhubAudioState")]
pub enum State {
    #[default]
    Loading,
    /// No PulseAudio-compatible server answers.
    Unavailable,
    Ready,
}

/// What a reload learns about one output or input.
struct Info {
    name: String,
    title: String,
    form_factor: String,
    bus: String,
    address: String,
}

mod imp {
    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::AudioManager)]
    pub struct AudioManager {
        #[property(get, builder(State::default()))]
        pub(super) state: Cell<State>,
        pub(super) outputs: gio::ListStore,
        pub(super) inputs: gio::ListStore,
        pub(super) mainloop: RefCell<Option<pulse_glib::Mainloop>>,
        pub(super) context: Rc<RefCell<Option<Context>>>,
        /// A reload is queued; change events come in bursts.
        pub(super) reload_queued: Cell<bool>,
        pub(super) known: RefCell<HashMap<String, AudioDevice>>,
    }

    impl Default for AudioManager {
        fn default() -> Self {
            Self {
                state: Cell::default(),
                outputs: gio::ListStore::new::<AudioDevice>(),
                inputs: gio::ListStore::new::<AudioDevice>(),
                mainloop: RefCell::default(),
                context: Rc::default(),
                reload_queued: Cell::default(),
                known: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AudioManager {
        const NAME: &'static str = "OpenhubAudioManager";
        type Type = super::AudioManager;
    }

    #[glib::derived_properties]
    impl ObjectImpl for AudioManager {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().connect();
        }
    }
}

glib::wrapper! {
    pub struct AudioManager(ObjectSubclass<imp::AudioManager>);
}

impl Default for AudioManager {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl AudioManager {
    pub fn outputs(&self) -> gio::ListStore {
        self.imp().outputs.clone()
    }

    pub fn inputs(&self) -> gio::ListStore {
        self.imp().inputs.clone()
    }

    fn set_state(&self, state: State) {
        if self.state() != state {
            self.imp().state.set(state);
            self.notify_state();
        }
    }

    /// Connects to the server, waiting for it if it isn't up yet. When it
    /// goes away (a PipeWire restart), connects again.
    fn connect(&self) {
        let imp = self.imp();
        let mainloop = pulse_glib::Mainloop::new(None).expect("libpulse GLib main loop");
        let mut proplist = Proplist::new().expect("proplist");
        let _ = proplist.set_str(pulse::proplist::properties::APPLICATION_NAME, "Openhub");
        let Some(mut context) = Context::new_with_proplist(&mainloop, "Openhub", &proplist) else {
            self.set_state(State::Unavailable);
            return;
        };
        let manager = self.downgrade();
        context.set_state_callback(Some(Box::new(move || {
            if let Some(manager) = manager.upgrade() {
                manager.on_state_changed();
            }
        })));
        if context.connect(None, FlagSet::NOFAIL, None).is_err() {
            self.set_state(State::Unavailable);
        }
        imp.context.replace(Some(context));
        imp.mainloop.replace(Some(mainloop));
    }

    fn on_state_changed(&self) {
        use pulse::context::State as Pulse;
        let state = self.imp().context.borrow().as_ref().map(Context::get_state);
        match state {
            Some(Pulse::Ready) => self.on_ready(),
            Some(Pulse::Failed | Pulse::Terminated) => {
                self.set_state(State::Unavailable);
                self.imp().outputs.remove_all();
                self.imp().inputs.remove_all();
                // Drop the dead context outside its own callback, then retry.
                let manager = self.downgrade();
                glib::timeout_add_seconds_local_once(2, move || {
                    if let Some(manager) = manager.upgrade() {
                        manager.imp().context.replace(None);
                        manager.connect();
                    }
                });
            }
            _ => {}
        }
    }

    fn on_ready(&self) {
        let manager = self.downgrade();
        let mut context = self.imp().context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        context.set_subscribe_callback(Some(Box::new(move |_, _, _| {
            if let Some(manager) = manager.upgrade() {
                manager.queue_reload();
            }
        })));
        let interest = InterestMaskSet::SINK | InterestMaskSet::SOURCE | InterestMaskSet::SERVER;
        context.subscribe(interest, |_| {});
        // The reload runs from idle, after this borrow of the context ends.
        self.queue_reload();
    }

    fn queue_reload(&self) {
        if self.imp().reload_queued.replace(true) {
            return;
        }
        let manager = self.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(manager) = manager.upgrade() {
                manager.imp().reload_queued.set(false);
                manager.reload();
            }
        });
    }

    /// Reads defaults, then outputs, then inputs, and applies them together.
    fn reload(&self) {
        let context = self.imp().context.borrow();
        let Some(context) = context.as_ref() else {
            return;
        };
        let introspect = Rc::new(context.introspect());
        let manager = self.downgrade();
        let introspect_next = introspect.clone();
        introspect.get_server_info(move |server| {
            let defaults = (
                server
                    .default_sink_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_owned(),
                server
                    .default_source_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_owned(),
            );
            let manager = manager.clone();
            let introspect_inputs = introspect_next.clone();
            let mut outputs = Vec::new();
            introspect_next.get_sink_info_list(move |result| match result {
                ListResult::Item(sink) => {
                    outputs.push(info(&sink.name, &sink.description, &sink.proplist))
                }
                ListResult::End => {
                    let outputs = std::mem::take(&mut outputs);
                    let manager = manager.clone();
                    let defaults = defaults.clone();
                    let mut inputs = Vec::new();
                    introspect_inputs.get_source_info_list(move |result| match result {
                        // A monitor is an output's loopback, not a microphone.
                        ListResult::Item(source) if source.monitor_of_sink.is_none() => {
                            inputs.push(info(&source.name, &source.description, &source.proplist));
                        }
                        ListResult::Item(_) => {}
                        ListResult::End => {
                            if let Some(manager) = manager.upgrade() {
                                manager.apply(&outputs, &std::mem::take(&mut inputs), &defaults);
                            }
                        }
                        ListResult::Error => {}
                    });
                }
                ListResult::Error => {}
            });
        });
    }

    fn apply(
        &self,
        outputs: &[Info],
        inputs: &[Info],
        (default_sink, default_source): &(String, String),
    ) {
        let imp = self.imp();
        let mut known = imp.known.borrow_mut();
        let mut next = HashMap::new();
        for (infos, store, default, output) in [
            (outputs, &imp.outputs, default_sink, true),
            (inputs, &imp.inputs, default_source, false),
        ] {
            let mut wanted = Vec::new();
            for info in infos {
                let device = known.remove(&info.name).unwrap_or_else(glib::Object::new);
                update(&device, info, output, &info.name == default);
                next.insert(info.name.clone(), device.clone());
                wanted.push(device);
            }
            let current: Vec<AudioDevice> = store.iter().flatten().collect();
            if current != wanted {
                store.splice(0, store.n_items(), &wanted);
            }
        }
        *known = next;
        drop(known);
        self.set_state(State::Ready);
    }
}

fn info(
    name: &Option<std::borrow::Cow<str>>,
    description: &Option<std::borrow::Cow<str>>,
    props: &Proplist,
) -> Info {
    let get = |key: &str| props.get_str(key).unwrap_or_default();
    let name = name.as_deref().unwrap_or_default().to_owned();
    Info {
        title: description
            .as_deref()
            .map_or_else(|| name.clone(), str::to_owned),
        name,
        form_factor: get("device.form_factor"),
        bus: get("device.bus"),
        address: get("api.bluez5.address"),
    }
}

fn update(device: &AudioDevice, info: &Info, output: bool, is_default: bool) {
    let set = |property: &str, value: &str| {
        if device.property::<String>(property) != value {
            device.set_property(property, value);
        }
    };
    set("name", &info.name);
    set("title", &info.title);
    set(
        "icon-name",
        icon_name(output, &info.form_factor, &info.name),
    );
    set("connection", connection(&info.bus));
    set("address", &info.address);
    if device.is_default() != is_default {
        device.set_is_default(is_default);
    }
}

/// From PulseAudio's `device.bus`; servers' own devices (such as an
/// effects app's sink) have none.
fn connection(bus: &str) -> &'static str {
    match bus {
        "bluetooth" => "Bluetooth",
        "usb" => "USB",
        "" => "Virtual",
        _ => "Built-in",
    }
}

fn icon_name(output: bool, form_factor: &str, name: &str) -> &'static str {
    match (output, form_factor) {
        (_, "headset" | "hands-free") => "audio-headset-symbolic",
        (true, "headphone") => "audio-headphones-symbolic",
        (false, "webcam") => "camera-web-symbolic",
        (true, _) if name.contains("HDMI") => "video-display-symbolic",
        (true, _) => "audio-speakers-symbolic",
        (false, _) => "audio-input-microphone-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_and_icons() {
        assert_eq!(connection("bluetooth"), "Bluetooth");
        assert_eq!(connection("usb"), "USB");
        assert_eq!(connection("pci"), "Built-in");
        assert_eq!(connection(""), "Virtual");
        let hdmi = "alsa_output.pci-0000_00_1f.3-platform-skl_hda_dsp_generic.HiFi__HDMI1__sink";
        assert_eq!(icon_name(true, "", hdmi), "video-display-symbolic");
        assert_eq!(
            icon_name(true, "headphone", "bluez_output.x"),
            "audio-headphones-symbolic"
        );
        assert_eq!(
            icon_name(false, "headset", "bluez_input.x"),
            "audio-headset-symbolic"
        );
        assert_eq!(
            icon_name(false, "", "alsa_input.usb-BY-V"),
            "audio-input-microphone-symbolic"
        );
    }
}
