//! A layout being edited, and the `ApplyMonitorsConfig` payload it becomes.
//! Changes collect here until Apply; Mutter then checks the layout
//! (`verify`) before keeping it (`persistent`), with GNOME's own "Keep these
//! display settings?" countdown.

use std::collections::HashMap;

use gtk::glib;
use gtk::glib::prelude::*;

use super::{Mode, Monitor, State};

/// `ApplyMonitorsConfig` methods.
pub const VERIFY: u32 = 0;
pub const PERSISTENT: u32 = 2;

/// Orientations, as Mutter numbers them, with GNOME Settings' names.
pub const ORIENTATIONS: [(u32, &str); 4] = [
    (0, "Landscape"),
    (1, "Portrait Right"),
    (3, "Portrait Left"),
    (2, "Landscape (Flipped)"),
];

/// `(ssa{sv})`: connector, mode ID, properties.
type MonitorConfig = (String, String, HashMap<String, glib::Variant>);
/// `(iiduba(ssa{sv}))`: x, y, scale, transform, primary, monitors.
type LogicalConfig = (i32, i32, f64, u32, bool, Vec<MonitorConfig>);

/// One logical monitor: a place on the desktop showing one monitor, or
/// several when mirrored.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: u32,
    pub primary: bool,
    /// (connector, mode ID).
    pub monitors: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub serial: u32,
    pub groups: Vec<Group>,
    layout_mode: u32,
    supports_layout_mode: bool,
    /// Each monitor's modes, to size groups and check choices.
    modes: HashMap<String, Vec<Mode>>,
}

impl Draft {
    pub fn new(state: &State) -> Self {
        let current = |connector: &str| {
            state
                .monitors
                .iter()
                .find(|m| m.connector == connector)
                .and_then(Monitor::current_mode)
                .map(|mode| mode.id.clone())
        };
        Self {
            serial: state.serial,
            groups: state
                .logical
                .iter()
                .map(|logical| Group {
                    x: logical.x,
                    y: logical.y,
                    scale: logical.scale,
                    transform: logical.transform,
                    primary: logical.primary,
                    monitors: logical
                        .connectors
                        .iter()
                        .filter_map(|c| Some((c.clone(), current(c)?)))
                        .collect(),
                })
                .filter(|group| !group.monitors.is_empty())
                .collect(),
            layout_mode: state.layout_mode,
            supports_layout_mode: state.supports_layout_mode,
            modes: state
                .monitors
                .iter()
                .map(|m| (m.connector.clone(), m.modes.clone()))
                .collect(),
        }
    }

    fn group_of(&self, connector: &str) -> Option<usize> {
        self.groups
            .iter()
            .position(|g| g.monitors.iter().any(|(c, _)| c == connector))
    }

    pub fn group(&self, connector: &str) -> Option<&Group> {
        self.group_of(connector).map(|i| &self.groups[i])
    }

    pub fn mode(&self, connector: &str) -> Option<&Mode> {
        let group = self.group(connector)?;
        let (_, id) = group.monitors.iter().find(|(c, _)| c == connector)?;
        self.modes.get(connector)?.iter().find(|m| &m.id == id)
    }

    pub fn enabled(&self, connector: &str) -> bool {
        self.group_of(connector).is_some()
    }

    /// Turns a monitor on (to the right of the others, at its preferred
    /// mode and scale) or off. The last monitor can't be turned off.
    pub fn set_enabled(&mut self, connector: &str, on: bool) {
        match (on, self.group_of(connector)) {
            (true, None) => {
                let Some(modes) = self.modes.get(connector) else {
                    return;
                };
                let Some(mode) = modes
                    .iter()
                    .find(|m| m.preferred && !m.variable)
                    .or(modes.first())
                else {
                    return;
                };
                self.groups.push(Group {
                    x: 0,
                    y: 0,
                    scale: mode.preferred_scale,
                    transform: 0,
                    primary: self.groups.is_empty(),
                    monitors: vec![(connector.to_owned(), mode.id.clone())],
                });
            }
            (false, Some(index))
                if self.groups.len() > 1 || self.groups[index].monitors.len() > 1 =>
            {
                let group = &mut self.groups[index];
                group.monitors.retain(|(c, _)| c != connector);
                if group.monitors.is_empty() {
                    let was_primary = self.groups.remove(index).primary;
                    if was_primary && let Some(first) = self.groups.first_mut() {
                        first.primary = true;
                    }
                }
            }
            _ => return,
        }
        self.repack();
    }

    /// Chooses a mode; the scale follows if the new mode doesn't offer it.
    pub fn set_mode(&mut self, connector: &str, id: &str) {
        let Some(mode) = self
            .modes
            .get(connector)
            .and_then(|m| m.iter().find(|m| m.id == id))
            .cloned()
        else {
            return;
        };
        let Some(index) = self.group_of(connector) else {
            return;
        };
        let group = &mut self.groups[index];
        for (c, mode_id) in &mut group.monitors {
            if c == connector {
                *mode_id = mode.id.clone();
            }
        }
        if !mode.scales.iter().any(|s| (s - group.scale).abs() < 1e-6) {
            group.scale = mode.preferred_scale;
        }
        self.repack();
    }

    pub fn set_scale(&mut self, connector: &str, scale: f64) {
        if let Some(index) = self.group_of(connector) {
            self.groups[index].scale = scale;
            self.repack();
        }
    }

    pub fn set_transform(&mut self, connector: &str, transform: u32) {
        if let Some(index) = self.group_of(connector) {
            self.groups[index].transform = transform;
            self.repack();
        }
    }

    /// A group's size on the desktop: its mode, turned by its orientation,
    /// divided by its scale in logical layout mode.
    fn size(&self, group: &Group) -> (i32, i32) {
        let Some((connector, id)) = group.monitors.first() else {
            return (0, 0);
        };
        let Some(mode) = self
            .modes
            .get(connector)
            .and_then(|m| m.iter().find(|m| &m.id == id))
        else {
            return (0, 0);
        };
        let (w, h) = if group.transform % 2 == 1 {
            (mode.height, mode.width)
        } else {
            (mode.width, mode.height)
        };
        if self.layout_mode == 1 {
            let scaled = |v: i32| (f64::from(v) / group.scale).round() as i32;
            (scaled(w), scaled(h))
        } else {
            (w, h)
        }
    }

    /// Keeps monitors that sit side by side edge to edge after a change of
    /// size, so the layout never has a gap or an overlap. Other layouts are
    /// left to Mutter's check; arranging them freely is the canvas's job.
    fn repack(&mut self) {
        if self.groups.iter().any(|g| g.y != 0) {
            return;
        }
        let mut order: Vec<usize> = (0..self.groups.len()).collect();
        order.sort_by_key(|&i| self.groups[i].x);
        let mut x = 0;
        for i in order {
            let width = self.size(&self.groups[i]).0;
            self.groups[i].x = x;
            x += width;
        }
    }

    /// The `(serial, method, logical monitors, properties)` arguments.
    pub fn payload(&self, method: u32) -> glib::Variant {
        let logical: Vec<LogicalConfig> = self
            .groups
            .iter()
            .map(|g| {
                let monitors = g
                    .monitors
                    .iter()
                    .map(|(c, id)| (c.clone(), id.clone(), HashMap::new()))
                    .collect();
                (g.x, g.y, g.scale, g.transform, g.primary, monitors)
            })
            .collect();
        let mut properties: HashMap<String, glib::Variant> = HashMap::new();
        if self.supports_layout_mode {
            properties.insert("layout-mode".into(), self.layout_mode.to_variant());
        }
        (self.serial, method, logical, properties).to_variant()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cjs_state() -> State {
        let path = format!(
            "{}/tests/fixtures/mutter/gnome50-laptop-and-asus.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        let ty = glib::VariantTy::new(super::super::state::SIGNATURE).unwrap();
        State::parse(&glib::Variant::parse(Some(ty), text.trim()).unwrap()).unwrap()
    }

    #[test]
    fn unchanged_layout_round_trips() {
        let draft = Draft::new(&cjs_state());
        assert_eq!(
            draft.payload(VERIFY).print(true).as_str(),
            "(uint32 1, uint32 0, [(0, 0, 1.0, uint32 0, true, [('DP-2', '1920x1080@144.001', @a{sv} {})]), \
             (1920, 0, 1.0, 0, false, [('eDP-1', '1920x1200@60.002', {})])], {'layout-mode': <uint32 1>})"
        );
    }

    #[test]
    fn a_smaller_mode_or_scale_keeps_monitors_edge_to_edge() {
        let mut draft = Draft::new(&cjs_state());
        // The ASUS at 1280×720 is narrower, so the panel moves left.
        draft.set_mode("DP-2", "1280x720@60.000");
        assert_eq!(draft.group("eDP-1").map(|g| g.x), Some(1280));
        // The panel at 125% doesn't move, but is 1536 wide; the ASUS at
        // 1920 again pushes it back.
        draft.set_scale("eDP-1", 1.25);
        draft.set_mode("DP-2", "1920x1080@144.001");
        assert_eq!(
            draft.group("eDP-1").map(|g| (g.x, g.scale)),
            Some((1920, 1.25))
        );
        // Turned to portrait, the ASUS is 1080 wide.
        draft.set_transform("DP-2", 1);
        assert_eq!(draft.group("eDP-1").map(|g| g.x), Some(1080));
    }

    #[test]
    fn a_mode_without_the_scale_takes_its_preferred_one() {
        let mut draft = Draft::new(&cjs_state());
        draft.set_scale("DP-2", 2.0);
        // 800×600 offers only 100%.
        draft.set_mode("DP-2", "800x600@60.317");
        assert_eq!(draft.group("DP-2").map(|g| g.scale), Some(1.0));
    }

    #[test]
    fn turning_monitors_off_and_on() {
        let mut draft = Draft::new(&cjs_state());
        draft.set_enabled("DP-2", false);
        assert!(!draft.enabled("DP-2"));
        // The panel is now alone, at 0,0, and primary.
        assert_eq!(
            draft.group("eDP-1").map(|g| (g.x, g.primary)),
            Some((0, true))
        );
        // The last monitor stays on.
        draft.set_enabled("eDP-1", false);
        assert!(draft.enabled("eDP-1"));
        // Back on: the ASUS at its preferred mode, to the right.
        draft.set_enabled("DP-2", true);
        assert_eq!(
            draft.mode("DP-2").map(|m| m.id.as_str()),
            Some("1920x1080@60.000")
        );
        assert_eq!(draft.group("DP-2").map(|g| g.x), Some(1920));
    }
}
