//! Mutter's `GetCurrentState` reply as plain types. It's a private API that
//! changes between GNOME versions, so every property is read optionally and
//! anything missing simply isn't shown.

use gtk::glib;

pub const SIGNATURE: &str = "(ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv})";

#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Mutter's serial for the set of connected monitors: it changes on a
    /// hotplug, not when the layout changes (seen on GNOME 50), and
    /// `ApplyMonitorsConfig` must send it back.
    pub serial: u32,
    pub monitors: Vec<Monitor>,
    pub logical: Vec<Logical>,
    /// 1: positions in logical pixels (scaled), 2: physical pixels.
    pub layout_mode: u32,
    /// Whether `layout-mode` may be sent back when applying.
    pub supports_layout_mode: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// The connector, such as `DP-2` or `eDP-1`.
    pub connector: String,
    pub vendor: String,
    pub product: String,
    pub serial: String,
    /// "ASUSTek COMPUTER INC 24\"", or "Built-in display".
    pub display_name: String,
    pub builtin: bool,
    pub modes: Vec<Mode>,
    /// The lowest rate variable refresh goes down to, where it's offered.
    pub min_refresh: Option<i32>,
    /// 0 is the default colour mode, 2 is BT.2100 (HDR).
    pub color_modes: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub id: String,
    pub width: i32,
    pub height: i32,
    pub refresh: f64,
    pub preferred_scale: f64,
    pub scales: Vec<f64>,
    pub current: bool,
    pub preferred: bool,
    /// A variable refresh rate mode ("+vrr").
    pub variable: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Logical {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// 0 normal, 1 to 3 rotated by 90° steps, 4 to 7 the same flipped.
    pub transform: u32,
    pub primary: bool,
    pub connectors: Vec<String>,
}

impl Monitor {
    pub fn current_mode(&self) -> Option<&Mode> {
        self.modes.iter().find(|mode| mode.current)
    }

    pub fn hdr_supported(&self) -> bool {
        self.color_modes.contains(&2)
    }

    pub fn vrr_supported(&self) -> bool {
        self.modes.iter().any(|mode| mode.variable)
    }
}

impl State {
    /// The logical monitor showing `connector`, if it's on.
    pub fn logical_for(&self, connector: &str) -> Option<&Logical> {
        self.logical
            .iter()
            .find(|logical| logical.connectors.iter().any(|c| c == connector))
    }

    pub fn parse(reply: &glib::Variant) -> Option<Self> {
        if reply.type_().as_str() != SIGNATURE {
            return None;
        }
        let monitors = reply.child_value(1);
        let logical = reply.child_value(2);
        let properties = glib::VariantDict::new(Some(&reply.child_value(3)));
        Some(Self {
            serial: reply.child_value(0).get()?,
            monitors: (0..monitors.n_children())
                .filter_map(|i| monitor(&monitors.child_value(i)))
                .collect(),
            logical: (0..logical.n_children())
                .filter_map(|i| logical_monitor(&logical.child_value(i)))
                .collect(),
            layout_mode: lookup(&properties, "layout-mode").unwrap_or(1),
            supports_layout_mode: lookup(&properties, "supports-changing-layout-mode")
                .unwrap_or(false),
        })
    }
}

fn lookup<T: glib::variant::FromVariant>(dict: &glib::VariantDict, key: &str) -> Option<T> {
    dict.lookup_value(key, None)?.get()
}

/// `(ssss)`: connector, vendor, product, serial.
fn spec(value: &glib::Variant) -> Option<(String, String, String, String)> {
    value.get()
}

fn monitor(value: &glib::Variant) -> Option<Monitor> {
    let (connector, vendor, product, serial) = spec(&value.child_value(0))?;
    let modes = value.child_value(1);
    let properties = glib::VariantDict::new(Some(&value.child_value(2)));
    Some(Monitor {
        connector,
        vendor,
        product,
        serial,
        display_name: lookup(&properties, "display-name").unwrap_or_default(),
        builtin: lookup(&properties, "is-builtin").unwrap_or(false),
        modes: (0..modes.n_children())
            .filter_map(|i| mode(&modes.child_value(i)))
            .collect(),
        min_refresh: lookup(&properties, "min-refresh-rate"),
        color_modes: lookup(&properties, "supported-color-modes").unwrap_or_default(),
    })
}

/// `(siiddada{sv})`: id, width, height, refresh, preferred scale, scales,
/// properties.
fn mode(value: &glib::Variant) -> Option<Mode> {
    let properties = glib::VariantDict::new(Some(&value.child_value(6)));
    let rate_mode: Option<String> = lookup(&properties, "refresh-rate-mode");
    Some(Mode {
        id: value.child_value(0).get()?,
        width: value.child_value(1).get()?,
        height: value.child_value(2).get()?,
        refresh: value.child_value(3).get()?,
        preferred_scale: value.child_value(4).get()?,
        scales: value.child_value(5).get()?,
        current: lookup(&properties, "is-current").unwrap_or(false),
        preferred: lookup(&properties, "is-preferred").unwrap_or(false),
        variable: rate_mode.as_deref() == Some("variable"),
    })
}

/// `(iiduba(ssss)a{sv})`: x, y, scale, transform, primary, monitors.
fn logical_monitor(value: &glib::Variant) -> Option<Logical> {
    let specs = value.child_value(5);
    Some(Logical {
        x: value.child_value(0).get()?,
        y: value.child_value(1).get()?,
        scale: value.child_value(2).get()?,
        transform: value.child_value(3).get()?,
        primary: value.child_value(4).get()?,
        connectors: (0..specs.n_children())
            .filter_map(|i| spec(&specs.child_value(i)).map(|s| s.0))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk::glib::prelude::*;

    fn fixture(name: &str) -> State {
        let path = format!(
            "{}/tests/fixtures/mutter/{name}.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        let ty = glib::VariantTy::new(SIGNATURE).unwrap();
        State::parse(&glib::Variant::parse(Some(ty), text.trim()).unwrap()).unwrap()
    }

    #[test]
    fn reads_a_laptop_with_an_external_monitor() {
        // GNOME 50: an ASUS VP249 at 144 Hz on DP-2, primary, left of the
        // laptop's own panel.
        let state = fixture("gnome50-laptop-and-asus");
        assert_eq!(state.serial, 1);
        assert_eq!(state.layout_mode, 1);
        let [asus, panel] = state.monitors.as_slice() else {
            panic!("two monitors");
        };
        assert_eq!((asus.connector.as_str(), asus.builtin), ("DP-2", false));
        assert_eq!(asus.display_name, "ASUSTek COMPUTER INC 24\"");
        let mode = asus.current_mode().unwrap();
        assert_eq!(
            (mode.width, mode.height, mode.refresh.round()),
            (1920, 1080, 144.0)
        );
        assert!(asus.hdr_supported());
        assert_eq!(asus.min_refresh, Some(48));
        assert!(panel.builtin && panel.vrr_supported());
        assert_eq!(panel.current_mode().map(|m| m.preferred_scale), Some(1.25));
        let left = state.logical_for("DP-2").unwrap();
        assert_eq!((left.x, left.y, left.primary), (0, 0, true));
        let right = state.logical_for("eDP-1").unwrap();
        assert_eq!((right.x, right.scale, right.primary), (1920, 1.0, false));
    }

    #[test]
    fn rejects_a_reply_of_another_shape() {
        assert_eq!(State::parse(&(1u32, "x").to_variant()), None);
    }
}
