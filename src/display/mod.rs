//! Displays: what Mutter (`org.gnome.Mutter.DisplayConfig`, the API GNOME
//! Settings and `gdctl` use) reports, and the monitors in sysfs as a
//! fallback where Mutter isn't running.

pub mod color;
pub mod config;
pub mod edid;
pub mod state;

use std::path::{Path, PathBuf};

use gtk::glib::prelude::*;
use gtk::{gio, glib};

pub use state::{Logical, Mode, Monitor, State};

const NAME: &str = "org.gnome.Mutter.DisplayConfig";
const PATH: &str = "/org/gnome/Mutter/DisplayConfig";

/// The current layout from Mutter, or `Err` when it isn't on the bus.
pub async fn current_state(bus: &gio::DBusConnection) -> Result<State, glib::Error> {
    let reply = bus
        .call_future(
            Some(NAME),
            PATH,
            NAME,
            "GetCurrentState",
            None,
            Some(glib::VariantTy::new(state::SIGNATURE).unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await?;
    State::parse(&reply)
        .ok_or_else(|| glib::Error::new(gio::IOErrorEnum::InvalidData, "Mutter's reply changed"))
}

/// Applies `draft`: Mutter first checks it (`verify`), so a layout it
/// would refuse is never applied, then keeps it (`persistent`), which has
/// GNOME Shell ask "Keep these display settings?" and revert after 20 s.
pub async fn apply(bus: &gio::DBusConnection, draft: &config::Draft) -> Result<(), glib::Error> {
    for method in [config::VERIFY, config::PERSISTENT] {
        bus.call_future(
            Some(NAME),
            PATH,
            NAME,
            "ApplyMonitorsConfig",
            Some(&draft.payload(method)),
            None,
            gio::DBusCallFlags::NONE,
            30_000,
        )
        .await?;
    }
    Ok(())
}

/// A backlight Mutter controls (the built-in panel, or a monitor the
/// `ddcci` kernel driver serves).
#[derive(Debug, Clone, PartialEq)]
pub struct Backlight {
    pub connector: String,
    pub min: i32,
    pub max: i32,
    pub value: i32,
}

/// Mutter's backlights (GNOME 49 and later), with the serial `SetBacklight`
/// wants back. `Err` where Mutter has no `Backlight` property.
pub async fn backlights(bus: &gio::DBusConnection) -> Result<(u32, Vec<Backlight>), glib::Error> {
    let reply = bus
        .call_future(
            Some(NAME),
            PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&(NAME, "Backlight").to_variant()),
            Some(glib::VariantTy::new("(v)").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await?;
    parse_backlights(&reply.child_value(0).as_variant().unwrap_or(reply)).ok_or_else(|| {
        glib::Error::new(gio::IOErrorEnum::InvalidData, "Mutter's backlights changed")
    })
}

/// `(uaa{sv})`: serial, then each backlight's connector, min, max, value.
fn parse_backlights(value: &glib::Variant) -> Option<(u32, Vec<Backlight>)> {
    if value.type_().as_str() != "(uaa{sv})" {
        return None;
    }
    let serial = value.child_value(0).get()?;
    let list = value.child_value(1);
    let backlights = (0..list.n_children())
        .filter_map(|i| {
            let dict = glib::VariantDict::new(Some(&list.child_value(i)));
            let get = |key| dict.lookup_value(key, None);
            Some(Backlight {
                connector: get("connector")?.get()?,
                min: get("min")?.get()?,
                max: get("max")?.get()?,
                value: get("value")?.get()?,
            })
        })
        .collect();
    Some((serial, backlights))
}

pub async fn set_backlight(
    bus: &gio::DBusConnection,
    serial: u32,
    connector: &str,
    value: i32,
) -> Result<(), glib::Error> {
    bus.call_future(
        Some(NAME),
        PATH,
        NAME,
        "SetBacklight",
        Some(&(serial, connector, value).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    )
    .await
    .map(drop)
}

/// Calls `changed` when GNOME Shell changes the brightness: the brightness
/// keys and Quick Settings on GNOME 50 go through GNOME Shell, which
/// changes the backlight without updating Mutter's `Backlight` property.
pub fn watch_shell_brightness(
    bus: &gio::DBusConnection,
    changed: impl Fn() + 'static,
) -> gio::SignalSubscription {
    bus.subscribe_to_signal(
        Some("org.gnome.Shell"),
        Some("org.gnome.Shell.Brightness"),
        Some("BrightnessChanged"),
        Some("/org/gnome/Shell/Brightness"),
        None,
        gio::DBusSignalFlags::NONE,
        move |_| changed(),
    )
}

/// The panel's real backlight level from the kernel, on the same scale as
/// Mutter's: the backlight device sits under its DRM connector
/// (`card1-eDP-1/intel_backlight/brightness`).
pub fn kernel_backlight(connector: &str) -> Option<i32> {
    kernel_backlight_in(Path::new("/sys/class/drm"), connector)
}

fn kernel_backlight_in(drm: &Path, connector: &str) -> Option<i32> {
    let card = std::fs::read_dir(drm).ok()?.flatten().find(|entry| {
        entry
            .file_name()
            .to_str()
            .and_then(|name| name.split_once('-'))
            .is_some_and(|(_, name)| name == connector)
    })?;
    std::fs::read_dir(card.path())
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            // A backlight class device: a folder with both files.
            if !path.join("max_brightness").exists() {
                return None;
            }
            std::fs::read_to_string(path.join("brightness"))
                .ok()?
                .trim()
                .parse()
                .ok()
        })
}

/// Calls `changed` when a backlight changes outside Openhub, such as with
/// the brightness keys or Quick Settings.
pub fn watch_backlights(
    bus: &gio::DBusConnection,
    changed: impl Fn() + 'static,
) -> gio::SignalSubscription {
    bus.subscribe_to_signal(
        Some(NAME),
        Some("org.freedesktop.DBus.Properties"),
        Some("PropertiesChanged"),
        Some(PATH),
        Some(NAME),
        gio::DBusSignalFlags::NONE,
        move |_| changed(),
    )
}

/// Calls `changed` whenever Mutter's layout changes. Dropping the returned
/// subscription stops it.
pub fn watch(bus: &gio::DBusConnection, changed: impl Fn() + 'static) -> gio::SignalSubscription {
    bus.subscribe_to_signal(
        Some(NAME),
        Some(NAME),
        Some("MonitorsChanged"),
        Some(PATH),
        None,
        gio::DBusSignalFlags::NONE,
        move |_| changed(),
    )
}

/// A connected monitor as sysfs shows it: enough for a list where Mutter
/// isn't running.
#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    /// `DP-2`, `eDP-1`, `HDMI-A-1`.
    pub name: String,
    pub edid: Option<edid::Edid>,
}

impl Connector {
    pub fn builtin(&self) -> bool {
        ["eDP", "LVDS", "DSI"]
            .iter()
            .any(|kind| self.name.starts_with(kind))
    }
}

pub fn connectors() -> Vec<Connector> {
    connectors_in(Path::new("/sys/class/drm"))
}

fn connectors_in(root: &Path) -> Vec<Connector> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, Connector)> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name();
            // card1-DP-2: card, then the connector's own name.
            let (_, name) = file.to_str()?.split_once('-')?;
            let path = entry.path();
            let status = std::fs::read_to_string(path.join("status")).ok()?;
            (status.trim() == "connected").then(|| {
                let edid = std::fs::read(path.join("edid")).ok();
                let connector = Connector {
                    name: name.to_owned(),
                    edid: edid.as_deref().and_then(edid::parse),
                };
                (path, connector)
            })
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found.into_iter().map(|(_, connector)| connector).collect()
}

/// The EDID of the monitor on `connector`, for the Details dialog.
pub fn edid_for(connector: &str) -> Option<edid::Edid> {
    connectors()
        .into_iter()
        .find(|c| c.name == connector)
        .and_then(|c| c.edid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_panels_kernel_backlight() {
        let root = std::env::temp_dir().join(format!("openhub-backlight-{}", std::process::id()));
        let backlight = root.join("card1-eDP-1/intel_backlight");
        std::fs::create_dir_all(&backlight).unwrap();
        std::fs::write(backlight.join("brightness"), "6996\n").unwrap();
        std::fs::write(backlight.join("max_brightness"), "15360\n").unwrap();
        std::fs::create_dir_all(root.join("card1-DP-2/i2c-13")).unwrap();
        let found = (
            kernel_backlight_in(&root, "eDP-1"),
            kernel_backlight_in(&root, "DP-2"),
        );
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, (Some(6996), None));
    }

    #[test]
    fn reads_mutters_backlights() {
        // As GNOME 50 reports CJ's built-in panel.
        let text = "(uint32 46, [{'connector': <'eDP-1'>, 'active': <true>, 'min': <153>, 'max': <15360>, 'value': <4713>}])";
        let value = glib::Variant::parse(None, text).unwrap();
        let (serial, backlights) = parse_backlights(&value).unwrap();
        assert_eq!(serial, 46);
        assert_eq!(
            backlights,
            [Backlight {
                connector: "eDP-1".into(),
                min: 153,
                max: 15360,
                value: 4713
            }]
        );
        assert_eq!(parse_backlights(&1u32.to_variant()), None);
    }

    #[test]
    fn lists_connected_monitors_from_sysfs() {
        let root = std::env::temp_dir().join(format!("openhub-drm-{}", std::process::id()));
        let fixture = |name| {
            std::fs::read(format!(
                "{}/tests/fixtures/edid/{name}.bin",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap()
        };
        for (node, status, edid) in [
            ("card1-DP-2", "connected", Some(fixture("asus-vp249"))),
            ("card1-HDMI-A-1", "disconnected", None),
            ("card1-eDP-1", "connected", Some(fixture("boe-nv140wum"))),
        ] {
            let dir = root.join(node);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("status"), format!("{status}\n")).unwrap();
            if let Some(edid) = edid {
                std::fs::write(dir.join("edid"), edid).unwrap();
            }
        }
        let found = connectors_in(&root);
        std::fs::remove_dir_all(&root).unwrap();
        let names: Vec<_> = found
            .iter()
            .map(|c| (c.name.as_str(), c.builtin()))
            .collect();
        assert_eq!(names, [("DP-2", false), ("eDP-1", true)]);
        assert_eq!(
            found[0].edid.as_ref().and_then(|e| e.name.as_deref()),
            Some("ASUS VP249")
        );
    }
}
