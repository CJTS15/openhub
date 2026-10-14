//! Mirrors UPower's batteries into GObjects the UI binds to. This is the only
//! code that talks to UPower; everything runs on the GTK main loop.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::Path;

use futures_util::stream::{self, StreamExt};
use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

use super::convert::{self, kind};
use super::proxies::{DeviceProxy, UPowerProxy};
use crate::model::PowerDevice;
use crate::watch::{ticks, watch};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "OpenhubPowerState")]
pub enum State {
    #[default]
    Loading,
    /// UPower isn't running or can't be reached.
    Unavailable,
    Ready,
}

mod imp {
    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::PowerManager)]
    pub struct PowerManager {
        #[property(get, builder(State::default()))]
        pub(super) state: Cell<State>,
        /// "On battery · 64% · 3 h 10 min left" and the like.
        #[property(get)]
        pub(super) status: RefCell<String>,
        /// The computer's overall battery percentage.
        #[property(get)]
        pub(super) level: Cell<f64>,
        /// False on a desktop with no battery; the status line is hidden then.
        #[property(get)]
        pub(super) has_battery: Cell<bool>,
        /// Batteries, chargers and UPS units powering this computer.
        pub(super) computer: gio::ListStore,
        /// Battery-powered peripherals, duplicates merged.
        pub(super) peripherals: gio::ListStore,
        /// Every UPower device and the task keeping it current.
        pub(super) devices: RefCell<HashMap<String, (PowerDevice, glib::JoinHandle<()>)>>,
        /// The task keeping the status line current.
        pub(super) status_watcher: RefCell<Option<glib::JoinHandle<()>>>,
    }

    impl Default for PowerManager {
        fn default() -> Self {
            Self {
                state: Cell::default(),
                status: RefCell::default(),
                level: Cell::default(),
                has_battery: Cell::default(),
                computer: gio::ListStore::new::<PowerDevice>(),
                peripherals: gio::ListStore::new::<PowerDevice>(),
                devices: RefCell::default(),
                status_watcher: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerManager {
        const NAME: &'static str = "OpenhubPowerManager";
        type Type = super::PowerManager;
    }

    #[glib::derived_properties]
    impl ObjectImpl for PowerManager {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj().clone();
            glib::spawn_future_local(async move { obj.run().await });
        }
    }
}

glib::wrapper! {
    pub struct PowerManager(ObjectSubclass<imp::PowerManager>);
}

impl PowerManager {
    /// The one manager for the app: the Power page and, later, the USB page
    /// read the same batteries.
    pub fn shared() -> Self {
        thread_local! {
            static SHARED: PowerManager = glib::Object::new();
        }
        SHARED.with(Clone::clone)
    }

    pub fn computer(&self) -> gio::ListStore {
        self.imp().computer.clone()
    }

    pub fn peripherals(&self) -> gio::ListStore {
        self.imp().peripherals.clone()
    }

    fn set_state(&self, state: State) {
        if self.state() != state {
            self.imp().state.set(state);
            self.notify_state();
        }
    }

    async fn run(&self) {
        if let Err(err) = self.watch_upower().await {
            glib::g_warning!("openhub", "Can't reach the system bus: {err}");
            self.set_state(State::Unavailable);
        }
    }

    /// Tracks UPower, starting over each time its name changes owner.
    async fn watch_upower(&self) -> zbus::Result<()> {
        let conn = zbus::Connection::system().await?;
        let upower = UPowerProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let mut owner_changes = upower.inner().receive_owner_changed().await?;
        loop {
            let session = glib::spawn_future_local(glib::clone!(
                #[strong(rename_to = this)]
                self,
                #[strong]
                conn,
                async move {
                    if let Err(err) = this.session(&conn).await {
                        glib::g_warning!("openhub", "UPower unavailable: {err}");
                        this.reset();
                        this.set_state(State::Unavailable);
                    }
                }
            ));
            let changed = owner_changes.next().await;
            session.abort();
            self.reset();
            self.set_state(State::Loading);
            if changed.is_none() {
                return Ok(());
            }
        }
    }

    fn reset(&self) {
        // Dropping a glib JoinHandle detaches the task; abort them explicitly.
        for (_, (_, watcher)) in self.imp().devices.take() {
            watcher.abort();
        }
        if let Some(watcher) = self.imp().status_watcher.take() {
            watcher.abort();
        }
        self.imp().computer.remove_all();
        self.imp().peripherals.remove_all();
    }

    async fn session(&self, conn: &zbus::Connection) -> zbus::Result<()> {
        let upower = UPowerProxy::builder(conn)
            .cache_properties(CacheProperties::Yes)
            .build()
            .await?;
        // Subscribe before listing so no change slips in between.
        let added = upower.receive_device_added().await?;
        let removed = upower.receive_device_removed().await?;
        for path in upower.enumerate_devices().await? {
            self.add(conn, path).await;
        }
        let display = DeviceProxy::builder(conn)
            .path(upower.get_display_device().await?)?
            .cache_properties(CacheProperties::Yes)
            .build()
            .await?;
        let status = self.watch_status(&upower, &display).await;
        self.imp().status_watcher.replace(Some(status));
        self.set_state(State::Ready);

        enum Event {
            Added(OwnedObjectPath),
            Removed(OwnedObjectPath),
        }
        let added = added.filter_map(|s| async move {
            Some(Event::Added(s.args().ok()?.device().to_owned().into()))
        });
        let removed = removed.filter_map(|s| async move {
            Some(Event::Removed(s.args().ok()?.device().to_owned().into()))
        });
        let mut events = stream::select(Box::pin(added), Box::pin(removed));
        while let Some(event) = events.next().await {
            match event {
                Event::Added(path) => self.add(conn, path).await,
                Event::Removed(path) => {
                    if let Some((_, watcher)) =
                        self.imp().devices.borrow_mut().remove(path.as_str())
                    {
                        watcher.abort();
                    }
                    self.relist();
                }
            }
        }
        Ok(())
    }

    /// Keeps the status line and overall level current.
    async fn watch_status(
        &self,
        upower: &UPowerProxy<'static>,
        display: &DeviceProxy<'static>,
    ) -> glib::JoinHandle<()> {
        let changes = vec![
            ticks(upower.receive_on_battery_changed().await),
            ticks(display.receive_percentage_changed().await),
            ticks(display.receive_state_changed().await),
            ticks(display.receive_time_to_empty_changed().await),
            ticks(display.receive_time_to_full_changed().await),
            ticks(display.receive_is_present_changed().await),
        ];
        let (upower, display) = (upower.clone(), display.clone());
        let manager = self.downgrade();
        watch(changes, move || {
            let Some(manager) = manager.upgrade() else {
                return;
            };
            let imp = manager.imp();
            let percentage = display
                .cached_percentage()
                .ok()
                .flatten()
                .unwrap_or_default();
            let has_battery = display
                .cached_is_present()
                .ok()
                .flatten()
                .unwrap_or_default()
                && display.cached_kind().ok().flatten() == Some(kind::BATTERY);
            let status = convert::status_line(
                upower
                    .cached_on_battery()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                percentage,
                display.cached_state().ok().flatten().unwrap_or_default(),
                display
                    .cached_time_to_empty()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
                display
                    .cached_time_to_full()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            );
            imp.level.set(percentage);
            imp.has_battery.set(has_battery);
            imp.status.replace(status);
            manager.notify_level();
            manager.notify_has_battery();
            manager.notify_status();
        })
    }

    async fn add(&self, conn: &zbus::Connection, path: OwnedObjectPath) {
        let watcher = match self.watch_device(conn, &path).await {
            Ok(watcher) => watcher,
            // The device can vanish while we build its proxy; skip it.
            Err(err) => {
                glib::g_warning!("openhub", "Can't watch {path}: {err}");
                return;
            }
        };
        if let Some((_, old)) = self
            .imp()
            .devices
            .borrow_mut()
            .insert(path.to_string(), watcher)
        {
            old.abort();
        }
        self.relist();
    }

    async fn watch_device(
        &self,
        conn: &zbus::Connection,
        path: &OwnedObjectPath,
    ) -> zbus::Result<(PowerDevice, glib::JoinHandle<()>)> {
        let proxy = DeviceProxy::builder(conn)
            .path(path.clone())?
            .cache_properties(CacheProperties::Yes)
            .build()
            .await?;
        let changes = vec![
            ticks(proxy.receive_model_changed().await),
            ticks(proxy.receive_online_changed().await),
            ticks(proxy.receive_is_present_changed().await),
            ticks(proxy.receive_state_changed().await),
            ticks(proxy.receive_percentage_changed().await),
        ];
        let native_path = proxy
            .cached_native_path()
            .ok()
            .flatten()
            .unwrap_or_default();
        let kind = proxy.cached_kind().ok().flatten().unwrap_or_default();
        let power_supply = proxy
            .cached_power_supply()
            .ok()
            .flatten()
            .unwrap_or_default();
        // A kernel battery's sysfs device shows whether it came over USB or Bluetooth.
        let sysfs = Path::new("/sys/class/power_supply")
            .join(&native_path)
            .join("device")
            .canonicalize()
            .ok();
        let connection = convert::connection(&native_path, sysfs.as_deref());
        let device: PowerDevice = glib::Object::builder()
            .property("path", path.as_str())
            .property("kind", kind)
            .property("power-supply", power_supply)
            .property(
                "serial",
                proxy.cached_serial().ok().flatten().unwrap_or_default(),
            )
            .property("bluez", native_path.starts_with("/org/bluez/"))
            .property("icon-name", convert::kind_icon(kind))
            .build();
        let manager = self.downgrade();
        let watcher = watch(
            changes,
            glib::clone!(
                #[strong]
                device,
                move || {
                    let model = proxy.cached_model().ok().flatten().unwrap_or_default();
                    // The computer's own battery reports a part code ("L22M3PG4") as its
                    // model; call it what it is and leave the code to the details.
                    let title = if power_supply || model.is_empty() {
                        convert::kind_name(kind).to_owned()
                    } else {
                        model
                    };
                    let percentage = proxy.cached_percentage().ok().flatten().unwrap_or_default();
                    let level = format!("{percentage:.0}%");
                    let state = convert::state_label(
                        proxy.cached_state().ok().flatten().unwrap_or_default(),
                    );
                    let subtitle = match (kind, power_supply) {
                        (kind::LINE_POWER, _) => {
                            let online = proxy.cached_online().ok().flatten().unwrap_or_default();
                            (if online { "Connected" } else { "Not connected" }).to_owned()
                        }
                        (_, true) if !state.is_empty() => format!("{level} · {state}"),
                        (_, true) => level,
                        (_, false) => format!("{level} · {connection}"),
                    };
                    device.set_title(title);
                    device.set_subtitle(subtitle);
                    device.set_level(if kind == kind::LINE_POWER {
                        -1.0
                    } else {
                        percentage
                    });
                    let present = proxy.cached_is_present().ok().flatten().unwrap_or(true);
                    if device.present() != present {
                        device.set_present(present);
                        if let Some(manager) = manager.upgrade() {
                            manager.relist();
                        }
                    }
                }
            ),
        );
        Ok((device, watcher))
    }

    /// Recomputes both lists from the tracked devices, replacing a list only
    /// when its members change so rows aren't rebuilt on every level tick.
    fn relist(&self) {
        let devices: Vec<PowerDevice> = self
            .imp()
            .devices
            .borrow()
            .values()
            .map(|(device, _)| device.clone())
            .collect();
        let paths: Vec<String> = devices.iter().map(PowerDevice::path).collect();
        let serials: Vec<String> = devices.iter().map(PowerDevice::serial).collect();
        let entries: Vec<convert::Entry> = devices
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.power_supply())
            .map(|(i, d)| convert::Entry {
                path: &paths[i],
                serial: &serials[i],
                bluez: d.bluez(),
            })
            .collect();
        let hidden = convert::hidden_duplicates(&entries);

        let mut computer: Vec<PowerDevice> = devices
            .iter()
            .filter(|d| d.power_supply() && (d.present() || d.kind() != kind::BATTERY))
            .cloned()
            .collect();
        // Batteries first, then UPS units, then chargers.
        computer.sort_by_key(|d| (computer_order(d.kind()), d.path()));
        let mut peripherals: Vec<PowerDevice> = devices
            .iter()
            .filter(|d| !d.power_supply() && !hidden.contains(&d.path()))
            .cloned()
            .collect();
        peripherals.sort_by_key(|d| (d.title().to_lowercase(), d.path()));
        replace_if_changed(&self.imp().computer, &computer);
        replace_if_changed(&self.imp().peripherals, &peripherals);
    }
}

fn computer_order(kind: u32) -> u8 {
    match kind {
        kind::BATTERY => 0,
        kind::UPS => 1,
        _ => 2,
    }
}

fn replace_if_changed(store: &gio::ListStore, wanted: &[PowerDevice]) {
    let current: Vec<PowerDevice> = store.iter::<PowerDevice>().flatten().collect();
    if current != wanted {
        store.splice(0, store.n_items(), wanted);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;

    use zbus::zvariant::{ObjectPath, Value};

    use super::*;

    const MOCK: &str = "org.freedesktop.DBus.Mock";
    const MAIN: &str = "/org/freedesktop/UPower";

    async fn wait_until(what: &str, check: impl Fn() -> bool) {
        for _ in 0..100 {
            if check() {
                return;
            }
            glib::timeout_future(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn mock_call<B>(
        conn: &zbus::Connection,
        path: &str,
        method: &str,
        body: &B,
    ) -> zbus::Message
    where
        B: zbus::export::serde::Serialize + zbus::zvariant::DynamicType,
    {
        conn.call_method(
            Some("org.freedesktop.UPower"),
            path,
            Some(MOCK),
            method,
            body,
        )
        .await
        .unwrap_or_else(|err| panic!("{method}: {err}"))
    }

    /// Adds a peripheral the way UPower does: the object with all its
    /// properties, then `DeviceAdded`.
    async fn add_peripheral(
        conn: &zbus::Connection,
        name: &str,
        native_path: &str,
        serial: &str,
    ) -> String {
        let path = format!("{MAIN}/devices/{name}");
        let props = HashMap::from([
            ("NativePath", Value::from(native_path)),
            ("Model", Value::from("MX Master 3S")),
            ("Serial", Value::from(serial)),
            ("Type", Value::from(convert::kind::MOUSE)),
            ("PowerSupply", Value::from(false)),
            ("IsPresent", Value::from(true)),
            ("State", Value::from(0u32)),
            ("Percentage", Value::from(80.0f64)),
        ]);
        let methods: Vec<(&str, &str, &str, &str)> = Vec::new();
        mock_call(
            conn,
            MAIN,
            "AddObject",
            &(&path, "org.freedesktop.UPower.Device", props, methods),
        )
        .await;
        let added = vec![Value::from(ObjectPath::try_from(path.as_str()).unwrap())];
        mock_call(
            conn,
            MAIN,
            "EmitSignal",
            &("org.freedesktop.UPower", "DeviceAdded", "o", added),
        )
        .await;
        path
    }

    /// Runs against python-dbusmock's UPower template on a private bus in
    /// place of the system bus; build-aux/test-upower.sh sets that up.
    #[test]
    #[ignore = "needs python-dbusmock; run build-aux/test-upower.sh"]
    fn upower_mock() {
        glib::MainContext::default().block_on(async {
            let conn = zbus::Connection::system().await.unwrap();
            let manager: PowerManager = glib::Object::new();
            wait_until("ready", || manager.state() == State::Ready).await;
            assert_eq!(manager.computer().n_items(), 0);

            // A laptop battery comes and goes.
            let battery: String = mock_call(
                &conn,
                MAIN,
                "AddDischargingBattery",
                &("mock_BAT", "Mock Battery", 30.0f64, 1200i64),
            )
            .await
            .body()
            .deserialize()
            .unwrap();
            wait_until("battery listed", || manager.computer().n_items() == 1).await;
            let listed: PowerDevice = manager.computer().item(0).and_downcast().unwrap();
            assert_eq!(
                (listed.title(), listed.subtitle()),
                ("Battery".into(), "30% · Discharging".into())
            );

            // The same mouse through the kernel and through BlueZ shows once, as the BlueZ one.
            add_peripheral(
                &conn,
                "battery_hidpp_battery_0",
                "hidpp_battery_0",
                "d5:79:24:d9:f8:be",
            )
            .await;
            let bluez = add_peripheral(
                &conn,
                "mouse_dev_D5",
                "/org/bluez/hci0/dev_D5",
                "D5:79:24:D9:F8:BE",
            )
            .await;
            wait_until("both tracked", || manager.imp().devices.borrow().len() == 3).await;
            assert_eq!(manager.peripherals().n_items(), 1);
            let mouse: PowerDevice = manager.peripherals().item(0).and_downcast().unwrap();
            assert_eq!(
                (mouse.path(), mouse.subtitle()),
                (bluez, "80% · Bluetooth".into())
            );

            mock_call(&conn, MAIN, "RemoveDevice", &(battery.as_str(),)).await;
            wait_until("battery gone", || manager.computer().n_items() == 0).await;
        });
    }
}
