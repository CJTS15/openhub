//! Typed proxies for the parts of ratbagd's D-Bus API (version 2) Openhub
//! uses; see libratbag's `doc/dbus.rst`.

use zbus::proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

#[proxy(
    interface = "org.freedesktop.ratbag1.Manager",
    default_service = "org.freedesktop.ratbag1",
    default_path = "/org/freedesktop/ratbag1"
)]
pub trait Manager {
    #[zbus(property, name = "APIVersion")]
    fn api_version(&self) -> zbus::Result<i32>;

    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

#[proxy(
    interface = "org.freedesktop.ratbag1.Device",
    default_service = "org.freedesktop.ratbag1"
)]
pub trait Device {
    /// `usb:1038:1836:0`: bus, vendor, product, version.
    #[zbus(property)]
    fn model(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn profiles(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Writes the changes to the mouse, asynchronously.
    fn commit(&self) -> zbus::Result<()>;
}

#[proxy(
    interface = "org.freedesktop.ratbag1.Profile",
    default_service = "org.freedesktop.ratbag1"
)]
pub trait Profile {
    #[zbus(property)]
    fn is_active(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn resolutions(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(property)]
    fn buttons(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(property)]
    fn report_rate(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn set_report_rate(&self, rate: u32) -> zbus::Result<()>;

    #[zbus(property)]
    fn report_rates(&self) -> zbus::Result<Vec<u32>>;
}

#[proxy(
    interface = "org.freedesktop.ratbag1.Resolution",
    default_service = "org.freedesktop.ratbag1"
)]
pub trait Resolution {
    #[zbus(property)]
    fn is_active(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn is_default(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn is_disabled(&self) -> zbus::Result<bool>;

    /// One number for both axes, or (x, y).
    #[zbus(property)]
    fn resolution(&self) -> zbus::Result<OwnedValue>;

    #[zbus(property)]
    fn set_resolution(&self, value: Value<'_>) -> zbus::Result<()>;

    /// The values `Resolution` accepts, lowest first.
    #[zbus(property)]
    fn resolutions(&self) -> zbus::Result<Vec<u32>>;
}

#[proxy(
    interface = "org.freedesktop.ratbag1.Button",
    default_service = "org.freedesktop.ratbag1"
)]
pub trait Button {
    #[zbus(property)]
    fn index(&self) -> zbus::Result<u32>;

    /// (action type, value) as a `(uv)` structure.
    #[zbus(property)]
    fn mapping(&self) -> zbus::Result<OwnedValue>;

    #[zbus(property)]
    fn set_mapping(&self, mapping: Value<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn action_types(&self) -> zbus::Result<Vec<u32>>;
}
