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
use pulse::def::BufferAttr;
use pulse::proplist::Proplist;
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, Stream};
use pulse::volume::{ChannelVolumes, Volume};

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
    /// Average over channels, as a fraction of 100 %.
    volume: f64,
    muted: bool,
    channels: u8,
    codec: String,
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
        /// Input level meters, by input name, while the page is showing.
        pub(super) meters: RefCell<HashMap<String, Rc<RefCell<Stream>>>>,
        pub(super) metering: Cell<bool>,
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
                meters: RefCell::default(),
                metering: Cell::default(),
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

        fn dispose(&self) {
            // Streams belong to the context, so they go first.
            for (_, meter) in self.meters.take() {
                close_meter(&meter);
            }
            // libpulse must free the context before the main loop it runs on;
            // the other order aborts in glib_defer_free when the window closes.
            if let Some(mut context) = self.context.borrow_mut().take() {
                context.disconnect();
            }
            self.mainloop.take();
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
                ListResult::Item(sink) => outputs.push(info(
                    &sink.name,
                    &sink.description,
                    &sink.proplist,
                    &sink.volume,
                    sink.mute,
                )),
                ListResult::End => {
                    let outputs = std::mem::take(&mut outputs);
                    let manager = manager.clone();
                    let defaults = defaults.clone();
                    let mut inputs = Vec::new();
                    introspect_inputs.get_source_info_list(move |result| match result {
                        // A monitor is an output's loopback, not a microphone.
                        ListResult::Item(source) if source.monitor_of_sink.is_none() => {
                            inputs.push(info(
                                &source.name,
                                &source.description,
                                &source.proplist,
                                &source.volume,
                                source.mute,
                            ));
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
        if imp.metering.get() {
            self.sync_meters();
        }
    }

    /// Makes `device` the default output or input.
    pub fn set_default(&self, device: &AudioDevice) {
        let mut context = self.imp().context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        let name = device.name();
        let done = |ok: bool| report("set the default device", ok);
        if device.output() {
            context.set_default_sink(&name, done);
        } else {
            context.set_default_source(&name, done);
        }
    }

    /// Sets every channel to `fraction` of 100 %.
    pub fn set_volume(&self, device: &AudioDevice, fraction: f64) {
        let context = self.imp().context.borrow();
        let Some(context) = context.as_ref() else {
            return;
        };
        let mut volumes = ChannelVolumes::default();
        let level = (fraction.clamp(0.0, 1.0) * f64::from(Volume::NORMAL.0)).round() as u32;
        let channels = u8::try_from(device.channels().clamp(1, 32)).unwrap_or(2);
        volumes.set(channels, Volume(level));
        let done: Box<dyn FnMut(bool)> = Box::new(|ok| report("change the volume", ok));
        let mut introspect = context.introspect();
        if device.output() {
            introspect.set_sink_volume_by_name(&device.name(), &volumes, Some(done));
        } else {
            introspect.set_source_volume_by_name(&device.name(), &volumes, Some(done));
        }
    }

    pub fn set_muted(&self, device: &AudioDevice, muted: bool) {
        let context = self.imp().context.borrow();
        let Some(context) = context.as_ref() else {
            return;
        };
        let done: Box<dyn FnMut(bool)> = Box::new(|ok| report("change muting", ok));
        let mut introspect = context.introspect();
        if device.output() {
            introspect.set_sink_mute_by_name(&device.name(), muted, Some(done));
        } else {
            introspect.set_source_mute_by_name(&device.name(), muted, Some(done));
        }
    }

    /// Runs the input level meters while the Audio page is showing.
    pub fn set_metering(&self, on: bool) {
        self.imp().metering.set(on);
        self.sync_meters();
    }

    /// One small peak-detect stream per input while metering, as pavucontrol
    /// does: 25 single-float readings a second.
    fn sync_meters(&self) {
        let imp = self.imp();
        let wanted: Vec<AudioDevice> = if imp.metering.get() {
            imp.inputs.iter().flatten().collect()
        } else {
            Vec::new()
        };
        let mut meters = imp.meters.borrow_mut();
        meters.retain(|name, meter| {
            let keep = wanted.iter().any(|device| &device.name() == name);
            if !keep {
                close_meter(meter);
            }
            keep
        });
        let mut context = imp.context.borrow_mut();
        let Some(context) = context.as_mut() else {
            return;
        };
        for device in wanted {
            if let std::collections::hash_map::Entry::Vacant(slot) = meters.entry(device.name()) {
                if let Some(meter) = peak_stream(context, &device) {
                    slot.insert(meter);
                }
            }
        }
    }
}

fn peak_stream(context: &mut Context, device: &AudioDevice) -> Option<Rc<RefCell<Stream>>> {
    let spec = Spec {
        format: Format::F32le,
        channels: 1,
        rate: 25,
    };
    let stream = Rc::new(RefCell::new(Stream::new(
        context,
        "Openhub level meter",
        &spec,
        None,
    )?));
    // The callback holds the stream weakly, or the two would keep each other alive.
    let weak_stream = Rc::downgrade(&stream);
    let weak_device = device.downgrade();
    stream
        .borrow_mut()
        .set_read_callback(Some(Box::new(move |_| {
            let (Some(stream), Some(device)) = (weak_stream.upgrade(), weak_device.upgrade())
            else {
                return;
            };
            let mut stream = stream.borrow_mut();
            let peak = match stream.peek() {
                Ok(PeekResult::Data(data)) => data
                    .chunks_exact(4)
                    .next_back()
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                Ok(PeekResult::Hole(_)) => None,
                Ok(PeekResult::Empty) | Err(_) => return,
            };
            let _ = stream.discard();
            if let Some(peak) = peak {
                device.set_peak(f64::from(peak).clamp(0.0, 1.0));
            }
        })));
    let attr = BufferAttr {
        maxlength: u32::MAX,
        tlength: u32::MAX,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: 4,
    };
    // No DONT_INHIBIT_AUTO_SUSPEND: an idle microphone is suspended and
    // would send nothing, so the meter keeps it awake while the page shows.
    let flags = StreamFlags::PEAK_DETECT | StreamFlags::ADJUST_LATENCY | StreamFlags::DONT_MOVE;
    if let Err(err) = stream
        .borrow_mut()
        .connect_record(Some(&device.name()), Some(&attr), flags)
    {
        glib::g_warning!("openhub", "Can't meter {}: {err}", device.name());
        return None;
    }
    Some(stream)
}

/// Stops a meter stream. libpulse can still have a read queued for it, so
/// the read callback is unset on the C side first; dropping the stream with
/// it set freed the Rust closure under a pending read and crashed.
fn close_meter(meter: &Rc<RefCell<Stream>>) {
    let mut stream = meter.borrow_mut();
    stream.set_read_callback(None);
    let _ = stream.disconnect();
}

fn report(what: &str, ok: bool) {
    if !ok {
        glib::g_warning!("openhub", "Couldn't {what}");
    }
}

fn info(
    name: &Option<std::borrow::Cow<str>>,
    description: &Option<std::borrow::Cow<str>>,
    props: &Proplist,
    volume: &ChannelVolumes,
    muted: bool,
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
        volume: f64::from(volume.avg().0) / f64::from(Volume::NORMAL.0),
        muted,
        channels: volume.len(),
        codec: codec_name(&get("api.bluez5.codec")),
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
    set("codec", &info.codec);
    if device.output() != output {
        device.set_output(output);
    }
    if device.is_default() != is_default {
        device.set_is_default(is_default);
    }
    // Small differences are rounding from the round trip through PulseAudio.
    if (device.volume() - info.volume).abs() > 0.001 {
        device.set_volume(info.volume);
    }
    if device.muted() != info.muted {
        device.set_muted(info.muted);
    }
    if device.channels() != u32::from(info.channels) {
        device.set_channels(u32::from(info.channels));
    }
}

/// PipeWire's `api.bluez5.codec` value, as people know it.
fn codec_name(codec: &str) -> String {
    match codec {
        "" => String::new(),
        "sbc" => "SBC".into(),
        "sbc_xq" => "SBC-XQ".into(),
        "aac" => "AAC".into(),
        "aptx" => "aptX".into(),
        "aptx_hd" => "aptX HD".into(),
        "aptx_ll" | "aptx_ll_duplex" => "aptX Low Latency".into(),
        "ldac" => "LDAC".into(),
        "lc3" | "lc3plus_h3" => "LC3".into(),
        "msbc" => "mSBC".into(),
        "cvsd" => "CVSD".into(),
        other => other.to_uppercase(),
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
    fn codec_names() {
        assert_eq!(codec_name("ldac"), "LDAC");
        assert_eq!(codec_name("aptx_hd"), "aptX HD");
        assert_eq!(codec_name("sbc_xq"), "SBC-XQ");
        assert_eq!(codec_name("opus_05"), "OPUS_05");
        assert_eq!(codec_name(""), "");
    }

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
