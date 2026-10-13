//! Typed proxies for the parts of the BlueZ D-Bus API Openhub uses.
//! `org.freedesktop.DBus.ObjectManager` comes from `zbus::fdo`.

use zbus::proxy;

#[proxy(interface = "org.bluez.Adapter1", default_service = "org.bluez")]
pub trait Adapter1 {
    fn start_discovery(&self) -> zbus::Result<()>;

    fn stop_discovery(&self) -> zbus::Result<()>;

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
