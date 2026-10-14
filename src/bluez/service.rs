//! Mirrors BlueZ's adapters and devices into GObjects the UI binds to, and
//! sends the UI's commands back.
//! This is the only code that talks to D-Bus; everything runs on the GTK main
//! loop via `glib::spawn_future_local`.

use std::cell::{Cell, OnceCell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::Future;
use std::time::Instant;

use futures_channel::mpsc;
use futures_util::stream::{self, StreamExt};
use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::{gio, glib};
use zbus::fdo::{InterfacesAdded, InterfacesRemoved, ObjectManagerProxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use super::agent::{self, Agent, PairingRequest};
use super::proxies::{Adapter1Proxy, AgentManager1Proxy, Battery1Proxy, Device1Proxy};
use crate::model::{Adapter, Device};
use crate::watch::{ticks, watch};

const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";

type PairingHandler = Box<dyn Fn(PairingRequest<Device>)>;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, glib::Enum)]
#[enum_type(name = "OpenhubBluetoothState")]
pub enum State {
    #[default]
    Loading,
    /// The BlueZ service isn't running or can't be reached.
    Unavailable,
    NoAdapter,
    Ready,
}

mod imp {
    use super::*;

    #[derive(glib::Properties)]
    #[properties(wrapper_type = super::BluetoothManager)]
    pub struct BluetoothManager {
        #[property(get, builder(State::default()))]
        pub(super) state: Cell<State>,
        /// Whether any adapter is scanning for devices.
        #[property(get)]
        pub(super) discovering: Cell<bool>,
        pub(super) adapters: gio::ListStore,
        pub(super) paired_devices: gio::ListStore,
        pub(super) nearby_devices: gio::ListStore,
        pub(super) conn: OnceCell<zbus::Connection>,
        pub(super) pairing_handler: RefCell<Option<PairingHandler>>,
    }

    impl Default for BluetoothManager {
        fn default() -> Self {
            Self {
                state: Cell::default(),
                discovering: Cell::default(),
                adapters: gio::ListStore::new::<Adapter>(),
                paired_devices: gio::ListStore::new::<Device>(),
                nearby_devices: gio::ListStore::new::<Device>(),
                conn: OnceCell::new(),
                pairing_handler: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BluetoothManager {
        const NAME: &'static str = "OpenhubBluetoothManager";
        type Type = super::BluetoothManager;
    }

    #[glib::derived_properties]
    impl ObjectImpl for BluetoothManager {
        fn constructed(&self) {
            self.parent_constructed();
            // Holds a strong ref for the app's lifetime; the window owns the only manager.
            let obj = self.obj().clone();
            glib::spawn_future_local(async move { obj.run().await });
        }
    }
}

glib::wrapper! {
    pub struct BluetoothManager(ObjectSubclass<imp::BluetoothManager>);
}

impl Default for BluetoothManager {
    fn default() -> Self {
        glib::Object::new()
    }
}

/// BlueZ objects tracked for one run of the BlueZ service.
#[derive(Default)]
struct Objects {
    adapters: HashMap<OwnedObjectPath, Adapter>,
    devices: HashMap<OwnedObjectPath, Device>,
    watchers: HashMap<(OwnedObjectPath, String), glib::JoinHandle<()>>,
}

impl Objects {
    /// Battery1 can arrive before Device1 on the same path, so either one creates the device.
    fn device(&mut self, path: &OwnedObjectPath) -> Device {
        self.devices
            .entry(path.clone())
            .or_insert_with(|| {
                glib::Object::builder()
                    .property("path", path.as_str())
                    .build()
            })
            .clone()
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        for (_, watcher) in self.watchers.drain() {
            watcher.abort();
        }
    }
}

enum Event {
    Added(InterfacesAdded),
    Removed(InterfacesRemoved),
}

impl BluetoothManager {
    /// `Adapter` items, one per local adapter.
    pub fn adapters(&self) -> gio::ListStore {
        self.imp().adapters.clone()
    }

    /// `Device` items for paired devices.
    pub fn paired_devices(&self) -> gio::ListStore {
        self.imp().paired_devices.clone()
    }

    /// `Device` items for unpaired devices that advertise a name.
    pub fn nearby_devices(&self) -> gio::ListStore {
        self.imp().nearby_devices.clone()
    }

    pub async fn set_powered(&self, adapter: &Adapter, powered: bool) -> zbus::Result<()> {
        self.adapter_proxy(adapter)
            .await?
            .set_powered(powered)
            .await
    }

    /// Starts or stops scanning on every powered adapter.
    pub async fn set_discovering(&self, discovering: bool) -> zbus::Result<()> {
        let any_powered = self
            .imp()
            .adapters
            .iter::<Adapter>()
            .any(|adapter| adapter.is_ok_and(|adapter| adapter.powered()));
        if discovering && !any_powered {
            return Err(zbus::Error::Failure("No adapter is powered".into()));
        }
        for adapter in self.imp().adapters.iter::<Adapter>() {
            let adapter = adapter.expect("adapters changed while iterating");
            if adapter.discovering() == discovering || (discovering && !adapter.powered()) {
                continue;
            }
            let proxy = self.adapter_proxy(&adapter).await?;
            if discovering {
                proxy.start_discovery().await?;
            } else {
                proxy.stop_discovery().await?;
            }
        }
        Ok(())
    }

    pub async fn connect_device(&self, device: &Device) -> zbus::Result<()> {
        let proxy = self.device_proxy(device).await?;
        timed("Connect", device, proxy.connect()).await
    }

    pub async fn disconnect_device(&self, device: &Device) -> zbus::Result<()> {
        let proxy = self.device_proxy(device).await?;
        timed("Disconnect", device, proxy.disconnect()).await
    }

    /// Pairs with `device`, asking the handler from `connect_pairing_request`
    /// whatever BlueZ needs, then trusts it so it can reconnect on its own.
    pub async fn pair_device(&self, device: &Device) -> zbus::Result<()> {
        let proxy = self.device_proxy(device).await?;
        timed("Pair", device, proxy.pair()).await?;
        timed("Trust", device, proxy.set_trusted(true)).await
    }

    pub async fn cancel_pairing(&self, device: &Device) -> zbus::Result<()> {
        self.device_proxy(device).await?.cancel_pairing().await
    }

    pub async fn set_trusted(&self, device: &Device, trusted: bool) -> zbus::Result<()> {
        self.device_proxy(device).await?.set_trusted(trusted).await
    }

    pub async fn set_blocked(&self, device: &Device, blocked: bool) -> zbus::Result<()> {
        self.device_proxy(device).await?.set_blocked(blocked).await
    }

    /// Removes the pairing and everything BlueZ knows about `device`.
    pub async fn forget_device(&self, device: &Device) -> zbus::Result<()> {
        let adapter = Adapter1Proxy::builder(self.connection()?)
            .path(device.adapter())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        adapter
            .remove_device(&ObjectPath::try_from(device.path())?)
            .await
    }

    /// Sets the one function that answers pairing requests, usually by asking the user.
    pub fn connect_pairing_request(&self, handler: impl Fn(PairingRequest<Device>) + 'static) {
        self.imp().pairing_handler.replace(Some(Box::new(handler)));
    }

    fn find_device(&self, path: &OwnedObjectPath) -> Option<Device> {
        let imp = self.imp();
        [&imp.paired_devices, &imp.nearby_devices]
            .into_iter()
            .flat_map(|store| store.iter::<Device>())
            .flatten()
            .find(|device| device.path() == path.as_str())
    }

    /// Serves the pairing agent on `conn` and passes its requests to the handler.
    async fn serve_agent(&self, conn: &zbus::Connection) -> zbus::Result<()> {
        let (requests, mut incoming) = mpsc::unbounded();
        conn.object_server()
            .at(agent::PATH, Agent { requests })
            .await?;
        let manager = self.downgrade();
        glib::spawn_future_local(async move {
            while let Some(request) = incoming.next().await {
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                // A request for a device we don't list is dropped, which rejects it.
                // Pair's time includes the user answering; this marks when they were asked.
                glib::g_debug!("openhub", "Pairing agent request received");
                let request = request.map_device(|path| manager.find_device(&path));
                if let (Some(request), Some(handler)) =
                    (request, &*manager.imp().pairing_handler.borrow())
                {
                    handler(request);
                }
            }
        });
        Ok(())
    }

    fn connection(&self) -> zbus::Result<&zbus::Connection> {
        self.imp()
            .conn
            .get()
            .ok_or_else(|| zbus::Error::Failure("Not connected to the system bus".into()))
    }

    async fn adapter_proxy(&self, adapter: &Adapter) -> zbus::Result<Adapter1Proxy<'static>> {
        Adapter1Proxy::builder(self.connection()?)
            .path(adapter.path())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    async fn device_proxy(&self, device: &Device) -> zbus::Result<Device1Proxy<'static>> {
        Device1Proxy::builder(self.connection()?)
            .path(device.path())?
            .cache_properties(CacheProperties::No)
            .build()
            .await
    }

    fn update_discovering(&self) {
        let discovering = self
            .imp()
            .adapters
            .iter::<Adapter>()
            .any(|adapter| adapter.is_ok_and(|adapter| adapter.discovering()));
        if self.discovering() != discovering {
            self.imp().discovering.set(discovering);
            self.notify_discovering();
        }
    }

    fn set_state(&self, state: State) {
        if self.state() != state {
            self.imp().state.set(state);
            self.notify_state();
        }
    }

    async fn run(&self) {
        if let Err(err) = self.watch_bluez().await {
            glib::g_warning!("openhub", "Can't reach the system bus: {err}");
            self.set_state(State::Unavailable);
        }
    }

    /// Tracks BlueZ, starting over each time its D-Bus name changes owner
    /// (the service started, stopped or restarted).
    async fn watch_bluez(&self) -> zbus::Result<()> {
        let conn = zbus::Connection::system().await?;
        // `run` is called once per manager, so the cell is always empty here.
        let _ = self.imp().conn.set(conn.clone());
        self.serve_agent(&conn).await?;
        let object_manager = ObjectManagerProxy::builder(&conn)
            .destination("org.bluez")?
            .path("/")?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let mut owner_changes = object_manager.inner().receive_owner_changed().await?;
        loop {
            let session = glib::spawn_future_local(glib::clone!(
                #[strong(rename_to = this)]
                self,
                #[strong]
                conn,
                #[strong]
                object_manager,
                async move {
                    if let Err(err) = this.session(&conn, &object_manager).await {
                        glib::g_warning!("openhub", "BlueZ unavailable: {err}");
                        this.set_state(State::Unavailable);
                    }
                }
            ));
            let changed = owner_changes.next().await;
            session.abort();
            self.imp().adapters.remove_all();
            self.imp().paired_devices.remove_all();
            self.imp().nearby_devices.remove_all();
            self.update_discovering();
            self.set_state(State::Loading);
            if changed.is_none() {
                return Ok(());
            }
        }
    }

    async fn session(
        &self,
        conn: &zbus::Connection,
        object_manager: &ObjectManagerProxy<'static>,
    ) -> zbus::Result<()> {
        // Subscribe before listing so no change slips in between.
        let added = object_manager.receive_interfaces_added().await?;
        let removed = object_manager.receive_interfaces_removed().await?;
        let mut objects = Objects::default();

        for (path, interfaces) in object_manager.get_managed_objects().await? {
            for interface in interfaces.keys() {
                self.add(&mut objects, conn, &path, interface).await;
            }
        }
        self.update_state();

        // Registered per BlueZ run; it forgets agents when it restarts.
        let agent_manager = AgentManager1Proxy::new(conn).await?;
        if let Err(err) = agent_manager
            .register_agent(&ObjectPath::try_from(agent::PATH)?, agent::CAPABILITY)
            .await
        {
            glib::g_warning!("openhub", "Can't register the pairing agent: {err}");
        }

        let mut events = stream::select(added.map(Event::Added), removed.map(Event::Removed));
        while let Some(event) = events.next().await {
            match event {
                Event::Added(signal) => {
                    let args = signal.args()?;
                    let path = OwnedObjectPath::from(args.object_path().clone());
                    for interface in args.interfaces_and_properties().keys() {
                        self.add(&mut objects, conn, &path, interface).await;
                    }
                }
                Event::Removed(signal) => {
                    let args = signal.args()?;
                    let path = OwnedObjectPath::from(args.object_path().clone());
                    for interface in args.interfaces().iter() {
                        self.remove(&mut objects, &path, interface);
                    }
                }
            }
            self.update_state();
        }
        Ok(())
    }

    fn update_state(&self) {
        self.set_state(if self.imp().adapters.n_items() == 0 {
            State::NoAdapter
        } else {
            State::Ready
        });
    }

    async fn add(
        &self,
        objects: &mut Objects,
        conn: &zbus::Connection,
        path: &OwnedObjectPath,
        interface: &str,
    ) {
        let watcher = match interface {
            ADAPTER => self.watch_adapter(objects, conn, path).await,
            DEVICE => self.watch_device(objects, conn, path).await,
            BATTERY => watch_battery(objects, conn, path).await,
            _ => return,
        };
        match watcher {
            Ok(watcher) => {
                let key = (path.clone(), interface.to_owned());
                if let Some(old) = objects.watchers.insert(key, watcher) {
                    old.abort();
                }
            }
            // The object can vanish while we build its proxy; skip it.
            Err(err) => glib::g_warning!("openhub", "Can't watch {interface} at {path}: {err}"),
        }
    }

    fn remove(&self, objects: &mut Objects, path: &OwnedObjectPath, interface: &str) {
        if let Some(watcher) = objects
            .watchers
            .remove(&(path.clone(), interface.to_owned()))
        {
            watcher.abort();
        }
        match interface {
            ADAPTER => {
                if let Some(adapter) = objects.adapters.remove(path) {
                    remove_item(&self.imp().adapters, &adapter);
                    self.update_discovering();
                }
            }
            DEVICE => {
                if let Some(device) = objects.devices.remove(path) {
                    remove_item(&self.imp().paired_devices, &device);
                    remove_item(&self.imp().nearby_devices, &device);
                }
            }
            BATTERY => {
                if let Some(device) = objects.devices.get(path) {
                    device.set_battery(-1);
                }
            }
            _ => {}
        }
    }

    async fn watch_adapter(
        &self,
        objects: &mut Objects,
        conn: &zbus::Connection,
        path: &OwnedObjectPath,
    ) -> zbus::Result<glib::JoinHandle<()>> {
        let proxy = Adapter1Proxy::builder(conn)
            .path(path.clone())?
            .cache_properties(CacheProperties::Yes)
            .build()
            .await?;
        let changes = vec![
            ticks(proxy.receive_alias_changed().await),
            ticks(proxy.receive_powered_changed().await),
            ticks(proxy.receive_discovering_changed().await),
        ];
        let adapter: Adapter = glib::Object::builder()
            .property("path", path.as_str())
            .build();
        objects.adapters.insert(path.clone(), adapter.clone());
        self.imp().adapters.append(&adapter);
        let manager = self.downgrade();
        Ok(watch(changes, move || {
            adapter.set_name(proxy.cached_alias().ok().flatten().unwrap_or_default());
            adapter.set_powered(proxy.cached_powered().ok().flatten().unwrap_or_default());
            adapter.set_discovering(
                proxy
                    .cached_discovering()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            );
            if let Some(manager) = manager.upgrade() {
                manager.update_discovering();
            }
        }))
    }

    async fn watch_device(
        &self,
        objects: &mut Objects,
        conn: &zbus::Connection,
        path: &OwnedObjectPath,
    ) -> zbus::Result<glib::JoinHandle<()>> {
        let proxy = Device1Proxy::builder(conn)
            .path(path.clone())?
            .cache_properties(CacheProperties::Yes)
            .build()
            .await?;
        let changes = vec![
            ticks(proxy.receive_name_changed().await),
            ticks(proxy.receive_alias_changed().await),
            ticks(proxy.receive_icon_changed().await),
            ticks(proxy.receive_paired_changed().await),
            ticks(proxy.receive_connected_changed().await),
            ticks(proxy.receive_trusted_changed().await),
            ticks(proxy.receive_blocked_changed().await),
        ];
        let device = objects.device(path);
        // Neither changes for the life of the object.
        device.set_address(proxy.cached_address().ok().flatten().unwrap_or_default());
        device.set_adapter(
            proxy
                .cached_adapter()
                .ok()
                .flatten()
                .map(|adapter| adapter.to_string())
                .unwrap_or_default(),
        );
        let paired_devices = self.paired_devices();
        let nearby_devices = self.nearby_devices();
        Ok(watch(changes, move || {
            device.set_name(proxy.cached_alias().ok().flatten().unwrap_or_default());
            device.set_icon(proxy.cached_icon().ok().flatten().unwrap_or_default());
            device.set_connected(proxy.cached_connected().ok().flatten().unwrap_or_default());
            device.set_trusted(proxy.cached_trusted().ok().flatten().unwrap_or_default());
            device.set_blocked(proxy.cached_blocked().ok().flatten().unwrap_or_default());
            let paired = proxy.cached_paired().ok().flatten().unwrap_or_default();
            device.set_paired(paired);
            // Unnamed devices are mostly anonymous LE beacons; leave them out.
            let named = proxy.cached_name().ok().flatten().is_some();
            place(&paired_devices, &device, paired);
            place(&nearby_devices, &device, !paired && named);
        }))
    }
}

async fn watch_battery(
    objects: &mut Objects,
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> zbus::Result<glib::JoinHandle<()>> {
    let proxy = Battery1Proxy::builder(conn)
        .path(path.clone())?
        .cache_properties(CacheProperties::Yes)
        .build()
        .await?;
    let changes = vec![ticks(proxy.receive_percentage_changed().await)];
    let device = objects.device(path);
    Ok(watch(changes, move || {
        let percentage = proxy.cached_percentage().ok().flatten();
        device.set_battery(percentage.map_or(-1, i32::from));
    }))
}

/// Awaits a BlueZ call and logs how long it took. Shown with
/// `G_MESSAGES_DEBUG=openhub`.
async fn timed<T>(
    step: &str,
    device: &Device,
    call: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    let start = Instant::now();
    let result = call.await;
    let outcome = match &result {
        Ok(_) => "done".to_owned(),
        Err(err) => format!("failed ({err})"),
    };
    glib::g_debug!(
        "openhub",
        "{step} {}: {outcome} after {:.1?}",
        device.name(),
        start.elapsed()
    );
    result
}

/// Connected devices first, then by name.
fn display_order(a: &glib::Object, b: &glib::Object) -> Ordering {
    let a = a.downcast_ref::<Device>().expect("device store");
    let b = b.downcast_ref::<Device>().expect("device store");
    b.connected()
        .cmp(&a.connected())
        .then_with(|| a.name().to_lowercase().cmp(&b.name().to_lowercase()))
}

/// Adds or removes `device` keeping `store` in display order. A device that
/// is out of order moves, which recreates its row.
fn place(store: &gio::ListStore, device: &Device, member: bool) {
    if let Some(position) = store.find(device) {
        if member && in_order(store, position) {
            return;
        }
        store.remove(position);
    }
    if member {
        store.insert_sorted(device, display_order);
    }
}

fn in_order(store: &gio::ListStore, position: u32) -> bool {
    let item = store.item(position).expect("position from find");
    let before = position.checked_sub(1).and_then(|p| store.item(p));
    let after = store.item(position + 1);
    before.is_none_or(|before| display_order(&before, &item).is_le())
        && after.is_none_or(|after| display_order(&item, &after).is_le())
}

fn remove_item(store: &gio::ListStore, item: &impl IsA<glib::Object>) {
    if let Some(position) = store.find(item) {
        store.remove(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, connected: bool) -> Device {
        glib::Object::builder()
            .property("name", name)
            .property("connected", connected)
            .build()
    }

    fn names(store: &gio::ListStore) -> Vec<String> {
        store.iter::<Device>().map(|d| d.unwrap().name()).collect()
    }

    #[test]
    fn devices_stay_in_display_order() {
        let store = gio::ListStore::new::<Device>();
        let (b, a, c) = (device("b", false), device("A", false), device("c", true));
        for d in [&b, &a, &c] {
            place(&store, d, true);
        }
        assert_eq!(names(&store), ["c", "A", "b"]);

        c.set_connected(false);
        place(&store, &c, true);
        assert_eq!(names(&store), ["A", "b", "c"]);

        b.set_connected(true);
        place(&store, &b, true);
        assert_eq!(names(&store), ["b", "A", "c"]);

        place(&store, &a, false);
        assert_eq!(names(&store), ["b", "c"]);
    }
}
