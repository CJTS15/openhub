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
    /// Each monitor's own mode, scale and orientation when the page loaded,
    /// to give back when it's turned on again or leaves a mirror. Mirrored
    /// monitors have none: their mode was the shared one.
    own: HashMap<String, (String, f64, u32)>,
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
        let mut draft = Self {
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
            own: HashMap::new(),
        };
        draft.own = draft
            .groups
            .iter()
            .filter(|g| g.monitors.len() == 1)
            .map(|g| {
                let (connector, id) = g.monitors[0].clone();
                (connector, (id, g.scale, g.transform))
            })
            .collect();
        draft
    }

    /// The mode, scale and orientation a monitor gets when it's turned on
    /// or leaves a mirror: its own from when the page loaded, else its
    /// preferred resolution at its fastest rate (Mutter's "preferred" mode
    /// is often 60 Hz on a faster monitor).
    fn own_settings(&self, connector: &str) -> Option<(String, f64, u32)> {
        if let Some(own) = self.own.get(connector) {
            return Some(own.clone());
        }
        let modes = self.modes.get(connector)?;
        let preferred = modes.iter().find(|m| m.preferred).or(modes.first())?;
        let mode = modes
            .iter()
            .filter(|m| !m.variable && (m.width, m.height) == (preferred.width, preferred.height))
            .max_by(|a, b| a.refresh.total_cmp(&b.refresh))?;
        Some((mode.id.clone(), mode.preferred_scale, 0))
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
                let Some((id, scale, transform)) = self.own_settings(connector) else {
                    return;
                };
                self.groups.push(Group {
                    x: 0,
                    y: 0,
                    scale,
                    transform,
                    primary: self.groups.is_empty(),
                    monitors: vec![(connector.to_owned(), id)],
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

    /// Each group's rectangle on the desktop: (x, y, width, height).
    pub fn rects(&self) -> Vec<(i32, i32, i32, i32)> {
        self.groups
            .iter()
            .map(|g| {
                let (w, h) = self.size(g);
                (g.x, g.y, w, h)
            })
            .collect()
    }

    pub fn set_primary(&mut self, connector: &str) {
        if let Some(index) = self.group_of(connector) {
            for (i, group) in self.groups.iter_mut().enumerate() {
                group.primary = i == index;
            }
        }
    }

    /// Moves a group to where it was dropped, snapped flush against the
    /// nearest neighbour. Returns false, and changes nothing, if no flush
    /// place avoids an overlap.
    pub fn move_to(&mut self, connector: &str, x: i32, y: i32) -> bool {
        let Some(index) = self.group_of(connector) else {
            return false;
        };
        let rects = self.rects();
        let (_, _, w, h) = rects[index];
        let others: Vec<(i32, i32, i32, i32)> = rects
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, r)| *r)
            .collect();
        if others.is_empty() {
            return false;
        }
        // Every flush place beside each neighbour, nearest to the drop first.
        let mut places: Vec<(i32, i32)> = Vec::new();
        for &(ox, oy, ow, oh) in &others {
            // Left or right of it, sliding along its side but still touching,
            // with nearly lined-up edges snapped together.
            let y_along = snap(y.clamp(oy - h + 1, oy + oh - 1), [oy, oy + oh - h]);
            let x_along = snap(x.clamp(ox - w + 1, ox + ow - 1), [ox, ox + ow - w]);
            for place in [
                (ox - w, y_along),
                (ox + ow, y_along),
                (x_along, oy - h),
                (x_along, oy + oh),
                // Edges lined up, which is what people usually mean.
                (ox - w, oy),
                (ox + ow, oy),
                (ox, oy - h),
                (ox, oy + oh),
            ] {
                places.push(place);
            }
        }
        places
            .sort_by_key(|&(px, py)| (i64::from(px - x).pow(2) + i64::from(py - y).pow(2), px, py));
        for (px, py) in places {
            let rect = (px, py, w, h);
            if others.iter().all(|other| !overlaps(rect, *other)) {
                self.groups[index].x = px;
                self.groups[index].y = py;
                self.normalize();
                return true;
            }
        }
        false
    }

    /// Moves the whole layout so its top-left corner is at 0,0, as Mutter
    /// requires.
    fn normalize(&mut self) {
        let min_x = self.groups.iter().map(|g| g.x).min().unwrap_or(0);
        let min_y = self.groups.iter().map(|g| g.y).min().unwrap_or(0);
        for group in &mut self.groups {
            group.x -= min_x;
            group.y -= min_y;
        }
    }

    /// No overlaps, and every monitor touching the rest: the layouts Mutter
    /// accepts. Checked before anything is sent.
    pub fn valid(&self) -> bool {
        let rects = self.rects();
        if rects.is_empty() || rects.iter().any(|r| r.2 <= 0 || r.3 <= 0) {
            return false;
        }
        for (i, a) in rects.iter().enumerate() {
            if rects[i + 1..].iter().any(|b| overlaps(*a, *b)) {
                return false;
            }
        }
        // Connected: walk the "touches" graph from the first monitor.
        let mut seen = vec![false; rects.len()];
        let mut stack = vec![0];
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            for (j, b) in rects.iter().enumerate() {
                if !seen[j] && touches(rects[i], *b) {
                    stack.push(j);
                }
            }
        }
        let min_x = rects.iter().map(|r| r.0).min();
        let min_y = rects.iter().map(|r| r.1).min();
        seen.iter().all(|s| *s) && min_x == Some(0) && min_y == Some(0)
    }

    /// The display modes, as GNOME Settings offers them.
    pub fn mode_kind(&self) -> Kind {
        match self.groups.as_slice() {
            [only] if only.monitors.len() > 1 => Kind::Mirror,
            [_] if self.modes.len() > 1 => Kind::Single,
            _ => Kind::Join,
        }
    }

    /// Every connected monitor showing the same picture, at the largest
    /// resolution they all have. False if they share none.
    pub fn mirror(&mut self) -> bool {
        let mut connectors: Vec<&String> = self.modes.keys().collect();
        connectors.sort();
        let sizes = |c: &String| -> Vec<(i32, i32)> {
            self.modes[c]
                .iter()
                .filter(|m| !m.variable)
                .map(|m| (m.width, m.height))
                .collect()
        };
        let Some(first) = connectors.first() else {
            return false;
        };
        let Some(&(w, h)) = sizes(first)
            .iter()
            .filter(|size| connectors.iter().all(|c| sizes(c).contains(size)))
            .max_by_key(|(w, h)| w * h)
        else {
            return false;
        };
        // Each monitor's fastest rate at that size; 100% is offered by all.
        let monitors = connectors
            .iter()
            .filter_map(|c| {
                let mode = self.modes[*c]
                    .iter()
                    .filter(|m| !m.variable && (m.width, m.height) == (w, h))
                    .max_by(|a, b| a.refresh.total_cmp(&b.refresh))?;
                Some(((*c).clone(), mode.id.clone()))
            })
            .collect();
        self.groups = vec![Group {
            x: 0,
            y: 0,
            scale: 1.0,
            transform: 0,
            primary: true,
            monitors,
        }];
        true
    }

    /// Only `connector` on.
    pub fn single(&mut self, connector: &str) {
        if !self.modes.contains_key(connector) {
            return;
        }
        let others: Vec<String> = self
            .modes
            .keys()
            .filter(|c| *c != connector)
            .cloned()
            .collect();
        // Out of a mirror first, so the chosen monitor keeps its own group.
        if self.mode_kind() == Kind::Mirror {
            self.join();
        }
        self.set_enabled(connector, true);
        for other in others {
            self.set_enabled(&other, false);
        }
        self.set_primary(connector);
    }

    /// Every connected monitor on, side by side.
    pub fn join(&mut self) {
        // Out of a mirror: each monitor gets its own group, at its own mode,
        // scale and orientation rather than the shared ones.
        if let [only] = self.groups.as_slice()
            && only.monitors.len() > 1
        {
            let group = self.groups.remove(0);
            for (i, (connector, shared)) in group.monitors.into_iter().enumerate() {
                let (id, scale, transform) =
                    self.own_settings(&connector)
                        .unwrap_or((shared, group.scale, group.transform));
                self.groups.push(Group {
                    x: 0,
                    y: 0,
                    scale,
                    transform,
                    primary: i == 0,
                    monitors: vec![(connector, id)],
                });
            }
        }
        let mut connectors: Vec<String> = self.modes.keys().cloned().collect();
        connectors.sort();
        for connector in connectors {
            if !self.enabled(&connector) {
                self.set_enabled(&connector, true);
            }
        }
        // Side by side from the left, keeping each monitor's order.
        for group in &mut self.groups {
            group.y = 0;
        }
        self.repack();
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Join,
    Mirror,
    Single,
}

/// How close, in pixels, an edge has to be to line up with a neighbour's.
const SNAP: i32 = 50;

fn snap(value: i32, edges: [i32; 2]) -> i32 {
    edges
        .into_iter()
        .filter(|edge| (value - edge).abs() <= SNAP)
        .min_by_key(|edge| (value - edge).abs())
        .unwrap_or(value)
}

/// Whether two rectangles share any area (touching edges don't count).
fn overlaps(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Whether two rectangles share part of an edge.
fn touches(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let side = (a.0 + a.2 == b.0 || b.0 + b.2 == a.0) && a.1 < b.1 + b.3 && b.1 < a.1 + a.3;
    let top = (a.1 + a.3 == b.1 || b.1 + b.3 == a.1) && a.0 < b.0 + b.2 && b.0 < a.0 + a.2;
    side || top
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
    fn dropped_monitors_snap_flush_and_stay_valid() {
        let mut draft = Draft::new(&cjs_state());
        assert!(draft.valid());
        // Dragged roughly below the ASUS, a little to the right: it lands
        // flush underneath, and the layout starts at 0,0.
        assert!(draft.move_to("eDP-1", 200, 1100));
        let panel = draft.group("eDP-1").unwrap();
        assert_eq!((panel.x, panel.y), (200, 1080));
        assert!(draft.valid());
        // Dropped near the left, it snaps to the ASUS's left side, lined up,
        // and the whole layout shifts so it starts at 0,0.
        assert!(draft.move_to("eDP-1", -1900, 30));
        assert_eq!(draft.group("eDP-1").map(|g| (g.x, g.y)), Some((0, 0)));
        assert_eq!(draft.group("DP-2").map(|g| (g.x, g.y)), Some((1920, 0)));
        assert!(draft.valid());
    }

    #[test]
    fn overlapping_or_gapped_layouts_are_invalid() {
        let mut draft = Draft::new(&cjs_state());
        draft.groups[1].x = 1000; // Over the ASUS.
        assert!(!draft.valid());
        draft.groups[1].x = 2000; // A gap between them.
        assert!(!draft.valid());
        draft.groups[1].x = 1920;
        draft.groups[0].x = 10; // Not starting at 0,0.
        assert!(!draft.valid());
    }

    #[test]
    fn leaving_mirror_or_single_gives_each_monitor_its_own_mode_back() {
        // The bug CJ found: Mirror then Join kept the shared 1920×1080 on the
        // panel, and a re-enabled ASUS came back at 60 Hz.
        for detour in ["mirror", "single"] {
            let mut draft = Draft::new(&cjs_state());
            match detour {
                "mirror" => {
                    draft.mirror();
                }
                _ => draft.single("eDP-1"),
            }
            draft.join();
            let mode = |c| draft.mode(c).map(|m| m.id.clone());
            assert_eq!(
                mode("DP-2").as_deref(),
                Some("1920x1080@144.001"),
                "{detour}"
            );
            assert_eq!(
                mode("eDP-1").as_deref(),
                Some("1920x1200@60.002"),
                "{detour}"
            );
            assert!(draft.valid(), "{detour}");
        }
    }

    #[test]
    fn a_monitor_off_at_load_comes_on_at_its_fastest_preferred_size() {
        let mut draft = Draft::new(&cjs_state());
        draft.own.remove("DP-2");
        draft.set_enabled("DP-2", false);
        draft.set_enabled("DP-2", true);
        // Mutter's preferred ASUS mode is 60 Hz; the same size goes to 144.
        assert_eq!(
            draft.mode("DP-2").map(|m| m.id.as_str()),
            Some("1920x1080@144.001")
        );
    }

    #[test]
    fn display_modes() {
        let mut draft = Draft::new(&cjs_state());
        assert_eq!(draft.mode_kind(), Kind::Join);
        draft.set_primary("eDP-1");
        assert_eq!(draft.group("DP-2").map(|g| g.primary), Some(false));
        // Mirror: one group, both at 1920×1080, the largest size they share,
        // each at its fastest rate there.
        assert!(draft.mirror());
        assert_eq!(draft.mode_kind(), Kind::Mirror);
        assert_eq!(
            draft.mode("DP-2").map(|m| m.id.as_str()),
            Some("1920x1080@144.001")
        );
        assert_eq!(
            draft.mode("eDP-1").map(|m| m.id.as_str()),
            Some("1920x1080@59.934")
        );
        assert!(draft.valid());
        // Single: only the panel.
        draft.single("eDP-1");
        assert_eq!(draft.mode_kind(), Kind::Single);
        assert!(!draft.enabled("DP-2") && draft.enabled("eDP-1"));
        // Join: both again, side by side.
        draft.join();
        assert_eq!(draft.mode_kind(), Kind::Join);
        assert!(draft.enabled("DP-2") && draft.valid());
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
        // Back on: the ASUS at its own 144 Hz again, to the right.
        draft.set_enabled("DP-2", true);
        assert_eq!(
            draft.mode("DP-2").map(|m| m.id.as_str()),
            Some("1920x1080@144.001")
        );
        assert_eq!(draft.group("DP-2").map(|g| g.x), Some(1920));
    }
}
