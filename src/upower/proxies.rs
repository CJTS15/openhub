//! Typed proxies for the parts of the UPower D-Bus API Openhub uses.

use zbus::proxy;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
pub trait UPower {
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// A composite of the computer's batteries, for the status line.
    fn get_display_device(&self) -> zbus::Result<OwnedObjectPath>;

    #[zbus(signal)]
    fn device_added(&self, device: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    fn device_removed(&self, device: ObjectPath<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
pub trait Device {
    /// The kernel power-supply name (`BAT0`, `hidpp_battery_0`) or, for
    /// batteries UPower gets from BlueZ, the BlueZ object path.
    #[zbus(property)]
    fn native_path(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn model(&self) -> zbus::Result<String>;

    /// For wireless peripherals, usually the Bluetooth address.
    #[zbus(property)]
    fn serial(&self) -> zbus::Result<String>;

    /// UPower's device type; see `convert::Kind`.
    #[zbus(property, name = "Type")]
    fn kind(&self) -> zbus::Result<u32>;

    /// Powers this computer (laptop battery, charger, UPS) rather than a peripheral.
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;

    /// For a charger: plugged in.
    #[zbus(property)]
    fn online(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;

    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;

    /// A coarse level (`convert::coarse_level`) for devices without a percentage.
    #[zbus(property)]
    fn battery_level(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn warning_level(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn vendor(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn technology(&self) -> zbus::Result<u32>;

    #[zbus(property)]
    fn capacity(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn energy(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn energy_full(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn energy_full_design(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn energy_rate(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn voltage(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn charge_cycles(&self) -> zbus::Result<i32>;

    /// The laptop exposes a charge limit (UPower 1.90 or newer).
    #[zbus(property)]
    fn charge_threshold_supported(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn charge_threshold_enabled(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn charge_end_threshold(&self) -> zbus::Result<u32>;

    /// Turns the charge limit on or off ("Preserve battery health").
    fn enable_charge_threshold(&self, enabled: bool) -> zbus::Result<()>;
}
