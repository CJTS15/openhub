//! Device firmware from fwupd (`org.freedesktop.fwupd` on the system bus),
//! the service GNOME Software and GNOME Firmware use. Read-only: versions
//! and whether the host's LVFS metadata has an update. Installing is left
//! to GNOME Software for now.

use gtk::glib::prelude::*;
use gtk::{gio, glib};

const NAME: &str = "org.freedesktop.fwupd";
/// `FWUPD_DEVICE_FLAG_UPDATABLE`. Devices updated only through another one
/// (`updatable-hidden`) don't have it.
const UPDATABLE: u64 = 1 << 1;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    pub icon: String,
    pub updatable: bool,
    /// Such as `USB\VID_046D&PID_C548`.
    pub instance_ids: Vec<String>,
}

impl Device {
    /// UEFI certificate stores (KEK, db, dbx): shown only with an update.
    pub fn certificate(&self) -> bool {
        self.icon == "application-certificate"
    }

    /// The fwupd entry for a USB device with this `vvvv:pppp` ID.
    pub fn matches_usb(&self, id: &str) -> bool {
        let Some((vendor, product)) = id.split_once(':') else {
            return false;
        };
        let wanted = format!(
            "USB\\VID_{}&PID_{}",
            vendor.to_ascii_uppercase(),
            product.to_ascii_uppercase()
        );
        self.instance_ids.iter().any(|i| i == &wanted)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub summary: String,
    /// Plain text, from fwupd's AppStream markup.
    pub notes: String,
}

pub async fn bus() -> Result<gio::DBusConnection, glib::Error> {
    gio::bus_get_future(gio::BusType::System).await
}

/// Every device fwupd knows, or `None` where it isn't running.
pub async fn devices(bus: &gio::DBusConnection) -> Option<Vec<Device>> {
    let reply = bus
        .call_future(
            Some(NAME),
            "/",
            NAME,
            "GetDevices",
            None,
            Some(glib::VariantTy::new("(aa{sv})").unwrap()),
            gio::DBusCallFlags::NONE,
            10_000,
        )
        .await
        .ok()?;
    Some(parse_devices(&reply.child_value(0)))
}

fn parse_devices(list: &glib::Variant) -> Vec<Device> {
    (0..list.n_children())
        .filter_map(|i| {
            let dict = glib::VariantDict::new(Some(&list.child_value(i)));
            let text = |key| {
                dict.lookup_value(key, None)
                    .and_then(|v| v.get::<String>())
                    .unwrap_or_default()
            };
            let list = |key| {
                dict.lookup_value(key, None)
                    .and_then(|v| v.get::<Vec<String>>())
                    .unwrap_or_default()
            };
            let flags = dict
                .lookup_value("Flags", None)
                .and_then(|v| v.get::<u64>())
                .unwrap_or(0);
            let id = text("DeviceId");
            (!id.is_empty()).then(|| Device {
                id,
                name: text("Name"),
                vendor: text("Vendor"),
                version: text("Version"),
                icon: list("Icon").into_iter().next().unwrap_or_default(),
                updatable: flags & UPDATABLE != 0,
                instance_ids: list("InstanceIds"),
            })
        })
        .collect()
}

/// The newest update for a device, if the host's metadata has one.
/// fwupd answers "nothing to do" as an error when there's none.
pub async fn upgrade(bus: &gio::DBusConnection, device: &str) -> Option<Release> {
    let reply = bus
        .call_future(
            Some(NAME),
            "/",
            NAME,
            "GetUpgrades",
            Some(&(device,).to_variant()),
            Some(glib::VariantTy::new("(aa{sv})").unwrap()),
            gio::DBusCallFlags::NONE,
            10_000,
        )
        .await
        .ok()?;
    parse_release(&reply.child_value(0))
}

fn parse_release(list: &glib::Variant) -> Option<Release> {
    let dict = glib::VariantDict::new(Some(&list.try_child_value(0)?));
    let text = |key| {
        dict.lookup_value(key, None)
            .and_then(|v| v.get::<String>())
            .unwrap_or_default()
    };
    Some(Release {
        version: text("Version"),
        summary: text("Summary"),
        notes: plain_text(&text("Description")),
    })
}

/// The devices the System page lists: those fwupd can update, with a
/// version. Certificate stores and unnamed parts ("UEFI Device Firmware",
/// no vendor, a bare number as version) only when they have an update.
pub fn shown(devices: &[Device], has_update: impl Fn(&Device) -> bool) -> Vec<&Device> {
    devices
        .iter()
        .filter(|d| d.updatable && !d.version.is_empty())
        .filter(|d| (!d.certificate() && !d.vendor.is_empty()) || has_update(d))
        .collect()
}

/// AppStream description markup (`<p>`, `<ul><li>`) as plain text.
fn plain_text(markup: &str) -> String {
    let mut text = String::new();
    let mut rest = markup;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            break;
        };
        match &rest[start + 1..start + end] {
            "li" => text.push_str("• "),
            "/p" | "/li" => text.push('\n'),
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    text.push_str(rest);
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Calls `changed` when fwupd's devices or metadata change.
pub fn watch(bus: &gio::DBusConnection, changed: impl Fn() + 'static) -> gio::SignalSubscription {
    bus.subscribe_to_signal(
        Some(NAME),
        Some(NAME),
        None,
        Some("/"),
        None,
        gio::DBusSignalFlags::NONE,
        move |_| changed(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_devices_and_filters_them() {
        // Trimmed from fwupd 2.x on CJ's laptop, plus a Logitech receiver.
        let text = "[{'DeviceId': <'50dff7'>, 'Name': <'System Firmware'>, 'Vendor': <'Lenovo'>, \
                    'Version': <'0.1.43'>, 'Icon': <['computer']>, 'Flags': <uint64 570425643>}, \
                    {'DeviceId': <'52ff2a'>, 'Name': <'Internal SPI Controller (BIOS)'>, \
                    'Version': <'R2AET68W(1.43)'>, 'Flags': <uint64 140875027972353>}, \
                    {'DeviceId': <'362301'>, 'Name': <'UEFI dbx'>, 'Version': <'20260707'>, \
                    'Icon': <['application-certificate']>, 'Flags': <uint64 4653133745619235>}, \
                    {'DeviceId': <'c0ffee'>, 'Name': <'UEFI Device Firmware'>, \
                    'Version': <'196608'>, 'Flags': <uint64 2>}, \
                    {'DeviceId': <'aa11bb'>, 'Name': <'Bolt Receiver'>, 'Vendor': <'Logitech'>, \
                    'Version': <'MPR05.03_B0010'>, 'Flags': <uint64 2>, \
                    'InstanceIds': <['USB\\\\VID_046D&PID_C548']>}]";
        let list =
            glib::Variant::parse(Some(glib::VariantTy::new("aa{sv}").unwrap()), text).unwrap();
        let devices = parse_devices(&list);
        assert_eq!(devices.len(), 5);
        assert!(devices[0].updatable && !devices[1].updatable);
        assert!(devices[4].matches_usb("046d:c548"));
        assert!(!devices[4].matches_usb("046d:c52b"));

        let names = |shown: Vec<&Device>| shown.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(shown(&devices, |_| false)),
            ["System Firmware", "Bolt Receiver"]
        );
        assert_eq!(
            names(shown(&devices, |d| d.id == "362301")),
            ["System Firmware", "UEFI dbx", "Bolt Receiver"]
        );
    }

    #[test]
    fn reads_a_release_and_its_notes() {
        let text = "[{'Version': <'0.1.45'>, 'Summary': <'Lenovo ThinkPad E14 Gen 5 System Firmware'>, \
                    'Description': <'<p>This stable release fixes:</p><ul><li>Fan noise &amp; heat</li>\
                    <li>Sleep on battery</li></ul>'>}]";
        let list =
            glib::Variant::parse(Some(glib::VariantTy::new("aa{sv}").unwrap()), text).unwrap();
        let release = parse_release(&list).unwrap();
        assert_eq!(release.version, "0.1.45");
        assert_eq!(
            release.notes,
            "This stable release fixes:\n• Fan noise & heat\n• Sleep on battery"
        );
        let empty =
            glib::Variant::parse(Some(glib::VariantTy::new("aa{sv}").unwrap()), "@aa{sv} []")
                .unwrap();
        assert_eq!(parse_release(&empty), None);
    }
}
