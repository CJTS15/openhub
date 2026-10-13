//! Typed proxies for the parts of the BlueZ D-Bus API Openhub uses.
//! `org.freedesktop.DBus.ObjectManager` comes from `zbus::fdo`.

use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

#[proxy(
    interface = "org.bluez.AgentManager1",
    default_service = "org.bluez",
    default_path = "/org/bluez"
)]
pub trait AgentManager1 {
    fn register_agent(&self, agent: &ObjectPath<'_>, capability: &str) -> zbus::Result<()>;
}

#[proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
pub trait Adapter1 {
    fn start_discovery(&self) -> zbus::Result<()>;

    fn stop_discovery(&self) -> zbus::Result<()>;

    fn remove_device(&self, device: &ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_powered(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn discovering(&self) -> zbus::Result<bool>;
}

#[proxy(interface = "org.bluez.Device1", default_service = "org.bluez")]
pub trait Device1 {
    fn connect(&self) -> zbus::Result<()>;

    fn disconnect(&self) -> zbus::Result<()>;

    fn pair(&self) -> zbus::Result<()>;

    fn cancel_pairing(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// The adapter this device belongs to.
    #[zbus(property)]
    fn adapter(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(property)]
    fn trusted(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_trusted(&self, value: bool) -> zbus::Result<()>;

    #[zbus(property)]
    fn blocked(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn set_blocked(&self, value: bool) -> zbus::Result<()>;

    /// The name the device advertises. Absent for many anonymous LE devices.
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn alias(&self) -> zbus::Result<String>;

    /// Freedesktop icon name such as `audio-headset`. Not every device has one.
    #[zbus(property)]
    fn icon(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn paired(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;
}

#[proxy(interface = "org.bluez.Battery1", default_service = "org.bluez")]
pub trait Battery1 {
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<u8>;
}
