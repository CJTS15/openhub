//! Displays: what Mutter (`org.gnome.Mutter.DisplayConfig`, the API GNOME
//! Settings and `gdctl` use) reports, and the monitors in sysfs as a
//! fallback where Mutter isn't running.

pub mod config;
pub mod edid;
pub mod state;

use std::path::{Path, PathBuf};

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
