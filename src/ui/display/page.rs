use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use super::canvas::{Canvas, Item};
use super::details_dialog;
use crate::display::config::{Draft, Kind, ORIENTATIONS};
use crate::display::{self, Mode, Monitor, State, edid};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct DisplayPage {
        pub page: adw::PreferencesPage,
        /// The groups currently shown, replaced on each change.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        pub subscription: RefCell<Option<gio::SignalSubscription>>,
        pub bus: RefCell<Option<gio::DBusConnection>>,
        pub state: RefCell<Option<State>>,
        /// The layout as edited, until Apply or Cancel.
        pub draft: RefCell<Option<Draft>>,
        pub render_queued: Cell<bool>,
        pub bar: gtk::Revealer,
        pub bar_label: gtk::Label,
        pub apply: gtk::Button,
        pub cancel: gtk::Button,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DisplayPage {
        const NAME: &'static str = "OpenhubDisplayPage";
        type Type = super::DisplayPage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for DisplayPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.build_bar();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            content.append(&self.bar);
            content.append(&self.page);
            self.page.set_vexpand(true);
            obj.set_child(Some(&content));
            glib::spawn_future_local(glib::clone!(
                #[weak]
                obj,
                async move { obj.start().await }
            ));
        }
    }

    impl WidgetImpl for DisplayPage {}
    impl BinImpl for DisplayPage {}
}

glib::wrapper! {
    /// The Displays page: every monitor's settings, as Mutter reports them,
    /// or a read-only list from sysfs where Mutter isn't running.
    pub struct DisplayPage(ObjectSubclass<imp::DisplayPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DisplayPage {
    /// The Apply / Cancel bar: layout changes wait here, as in GNOME
    /// Settings, since a wrong one can leave a screen unusable.
    fn build_bar(&self) {
        let imp = self.imp();
        imp.bar_label.set_label("Apply the new display settings?");
        imp.bar_label.set_hexpand(true);
        imp.bar_label.set_xalign(0.0);
        imp.bar_label.set_wrap(true);
        imp.cancel.set_label("Cancel");
        imp.apply.set_label("Apply");
        imp.apply.add_css_class("suggested-action");
        let bar = gtk::Box::builder()
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();
        bar.append(&imp.bar_label);
        bar.append(&imp.cancel);
        bar.append(&imp.apply);
        imp.bar.set_child(Some(&bar));
        imp.cancel.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.cancel()
        ));
        imp.apply.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |_| page.apply()
        ));
    }

    async fn start(&self) {
        let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
            self.show_fallback();
            return;
        };
        // Mutter signals each layout change; nothing polls.
        let subscription = display::watch(
            &bus,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                #[strong]
                bus,
                move || {
                    glib::spawn_future_local(glib::clone!(
                        #[weak]
                        page,
                        #[strong]
                        bus,
                        async move { page.reload(&bus).await }
                    ));
                }
            ),
        );
        self.imp().subscription.replace(Some(subscription));
        self.imp().bus.replace(Some(bus.clone()));
        self.reload(&bus).await;
    }

    async fn reload(&self, bus: &gio::DBusConnection) {
        match display::current_state(bus).await {
            Ok(state) => {
                // The serial stays the same across layout changes, so the
                // whole layout is compared: a revert in GNOME Shell's dialog
                // only differs in where the monitors are.
                if self.imp().state.borrow().as_ref() != Some(&state) {
                    self.show_state(state);
                }
            }
            Err(_) => self.show_fallback(),
        }
    }

    fn replace_groups(&self, groups: Vec<adw::PreferencesGroup>) {
        let imp = self.imp();
        for group in imp.groups.take() {
            imp.page.remove(&group);
        }
        for group in &groups {
            imp.page.add(group);
        }
        imp.groups.replace(groups);
    }

    /// A new layout from Mutter: any edits are dropped.
    fn show_state(&self, state: State) {
        let imp = self.imp();
        imp.draft.replace(Some(Draft::new(&state)));
        imp.state.replace(Some(state));
        imp.apply.set_sensitive(true);
        imp.cancel.set_sensitive(true);
        imp.bar_label.set_label("Apply the new display settings?");
        self.render();
    }

    fn cancel(&self) {
        let imp = self.imp();
        let draft = imp.state.borrow().as_ref().map(Draft::new);
        imp.draft.replace(draft);
        self.render();
    }

    fn apply(&self) {
        let imp = self.imp();
        let (Some(bus), Some(draft)) = (imp.bus.borrow().clone(), imp.draft.borrow().clone())
        else {
            return;
        };
        // Never sent with a gap or an overlap; Mutter would refuse it anyway.
        if !draft.valid() {
            imp.bar_label
                .set_label("These displays overlap or don't touch. Move them together first.");
            return;
        }
        imp.apply.set_sensitive(false);
        imp.cancel.set_sensitive(false);
        imp.bar_label.set_label("Applying…");
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                match display::apply(&bus, &draft).await {
                    // Mutter sends no MonitorsChanged for a layout it finds
                    // unchanged, so the page resets from a fresh read rather
                    // than waiting for one. A later revert in GNOME Shell's
                    // dialog arrives as a signal, as before.
                    Ok(()) => {
                        if let Ok(state) = display::current_state(&bus).await {
                            page.show_state(state);
                        }
                    }
                    Err(error) => {
                        let imp = page.imp();
                        imp.bar_label
                            .set_label(&format!("Couldn't apply: {}", error.message()));
                        imp.apply.set_sensitive(true);
                        imp.cancel.set_sensitive(true);
                    }
                }
            }
        ));
    }

    /// Edits the draft, then redraws once the change handler has returned
    /// (the rows it would replace include the one being changed).
    fn edit(&self, change: impl FnOnce(&mut Draft)) {
        let imp = self.imp();
        if let Some(draft) = imp.draft.borrow_mut().as_mut() {
            change(draft);
        }
        if !imp.render_queued.replace(true) {
            glib::idle_add_local_once(glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    page.imp().render_queued.set(false);
                    page.render();
                }
            ));
        }
    }

    fn render(&self) {
        let imp = self.imp();
        let (Some(state), Some(draft)) = (imp.state.borrow().clone(), imp.draft.borrow().clone())
        else {
            return;
        };
        imp.bar.set_reveal_child(draft != Draft::new(&state));
        // Built-in first, then the rest in Mutter's order; numbered so.
        let mut monitors: Vec<&Monitor> = state.monitors.iter().collect();
        monitors.sort_by_key(|monitor| !monitor.builtin);
        let several = monitors.len() > 1;
        let mut groups = Vec::new();
        if several {
            groups.push(self.mode_group(&draft, &monitors));
            if draft.mode_kind() == Kind::Join {
                groups.push(self.arrangement_group(&draft, &monitors));
            }
        }
        if draft.mode_kind() == Kind::Mirror {
            // Mirrored monitors share one picture, so their settings go
            // together; Mutter picks the shared resolution.
            let size = draft
                .groups
                .first()
                .and_then(|g| g.monitors.first())
                .and_then(|(c, _)| draft.mode(c))
                .map(|m| format!("{} × {}", m.width, m.height))
                .unwrap_or_default();
            groups.push(
                adw::PreferencesGroup::builder()
                    .title("Mirrored Displays")
                    .description(format!("Every display shows the same picture, at {size}."))
                    .build(),
            );
        } else {
            groups.extend(
                monitors
                    .iter()
                    .map(|monitor| self.monitor_group(&state, &draft, monitor, several)),
            );
        }
        self.replace_groups(groups);
    }

    /// Join, Mirror or a single display, and which one is primary or shown.
    fn mode_group(&self, draft: &Draft, monitors: &[&Monitor]) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title("Display Mode")
            .build();
        let kinds = [Kind::Join, Kind::Mirror, Kind::Single];
        let kind = draft.mode_kind();
        let selected = kinds.iter().position(|k| *k == kind).unwrap_or(0);
        let names: Vec<String> = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| {
                format!(
                    "{} {}",
                    i + 1,
                    title(m, display::edid_for(&m.connector).as_ref())
                )
            })
            .collect();
        let connectors: Vec<String> = monitors.iter().map(|m| m.connector.clone()).collect();
        let first = connectors.first().cloned().unwrap_or_default();
        group.add(&self.combo(
            "Mode",
            &["Join Displays", "Mirror", "Single Display"],
            selected,
            move |page, index| match kinds.get(index) {
                Some(Kind::Join) => page.edit(Draft::join),
                Some(Kind::Mirror) => page.edit(|d| {
                    d.mirror();
                }),
                Some(Kind::Single) => page.edit(|d| d.single(&first)),
                None => {}
            },
        ));
        let labels: Vec<&str> = names.iter().map(String::as_str).collect();
        match kind {
            Kind::Join => {
                let primary = connectors
                    .iter()
                    .position(|c| draft.group(c).is_some_and(|g| g.primary))
                    .unwrap_or(0);
                let c = connectors.clone();
                group.add(
                    &self.combo("Primary Display", &labels, primary, move |page, index| {
                        if let Some(connector) = c.get(index) {
                            page.edit(|d| d.set_primary(connector));
                        }
                    }),
                );
            }
            Kind::Single => {
                let shown = connectors
                    .iter()
                    .position(|c| draft.enabled(c))
                    .unwrap_or(0);
                let c = connectors.clone();
                group.add(&self.combo("Display", &labels, shown, move |page, index| {
                    if let Some(connector) = c.get(index) {
                        page.edit(|d| d.single(connector));
                    }
                }));
            }
            Kind::Mirror => {}
        }
        group
    }

    /// The monitors drawn to scale, to drag into place, and Identify.
    fn arrangement_group(&self, draft: &Draft, monitors: &[&Monitor]) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title("Arrangement")
            .description("Drag displays to match how they're placed on your desk.")
            .build();
        let identify = gtk::Button::builder()
            .label("Identify")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let numbers: Vec<(String, usize)> = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| (m.connector.clone(), i + 1))
            .collect();
        identify.connect_clicked(move |button| identify_displays(button, &numbers));
        group.set_header_suffix(Some(&identify));
        let rects = draft.rects();
        let items = draft
            .groups
            .iter()
            .zip(rects)
            .filter_map(|(g, rect)| {
                let (connector, _) = g.monitors.first()?;
                let (number, monitor) = monitors
                    .iter()
                    .enumerate()
                    .find(|(_, m)| &m.connector == connector)?;
                Some(Item {
                    connector: connector.clone(),
                    number: number + 1,
                    name: title(monitor, display::edid_for(connector).as_ref()),
                    rect,
                    primary: g.primary,
                })
            })
            .collect();
        let canvas = Canvas::default();
        canvas.set_items(items);
        canvas.connect_dropped(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |connector, x, y| {
                // A drop with no flush place snaps back on the redraw.
                let connector = connector.to_owned();
                page.edit(|d| {
                    d.move_to(&connector, x, y);
                });
            }
        ));
        let card = gtk::Box::builder().css_classes(["card"]).build();
        card.append(&canvas);
        group.add(&card);
        group
    }

    fn monitor_group(
        &self,
        state: &State,
        draft: &Draft,
        monitor: &Monitor,
        several: bool,
    ) -> adw::PreferencesGroup {
        let connector = monitor.connector.clone();
        let edid = display::edid_for(&connector);
        let group = adw::PreferencesGroup::builder()
            .title(title(monitor, edid.as_ref()))
            .description(subtitle(state, monitor))
            .build();
        let details = gtk::Button::builder()
            .label("Details")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (shown, logical) = (monitor.clone(), state.logical_for(&connector).cloned());
        details.connect_clicked(move |button| {
            let edid = display::edid_for(&shown.connector);
            details_dialog::new(&shown, logical.as_ref(), edid.as_ref()).present(Some(button));
        });
        group.set_header_suffix(Some(&details));

        let enabled = draft.enabled(&connector);
        if several {
            let row = adw::SwitchRow::builder()
                .title("Display")
                .active(enabled)
                .build();
            let c = connector.clone();
            row.connect_active_notify(glib::clone!(
                #[weak(rename_to = page)]
                self,
                move |row| page.edit(|d| d.set_enabled(&c, row.is_active()))
            ));
            group.add(&row);
        }
        let (Some(mode), Some(placed)) = (
            draft.mode(&connector).cloned(),
            draft.group(&connector).cloned(),
        ) else {
            return group;
        };

        // Orientation.
        let labels: Vec<&str> = ORIENTATIONS.iter().map(|(_, name)| *name).collect();
        let selected = ORIENTATIONS
            .iter()
            .position(|(t, _)| *t == placed.transform)
            .unwrap_or(0);
        let c = connector.clone();
        group.add(
            &self.combo("Orientation", &labels, selected, move |page, index| {
                if let Some((transform, _)) = ORIENTATIONS.get(index) {
                    page.edit(|d| d.set_transform(&c, *transform));
                }
            }),
        );

        // Resolution, then the rates that resolution offers.
        let fixed: Vec<&Mode> = monitor.modes.iter().filter(|m| !m.variable).collect();
        let mut sizes: Vec<(i32, i32)> = Vec::new();
        for m in &fixed {
            if !sizes.contains(&(m.width, m.height)) {
                sizes.push((m.width, m.height));
            }
        }
        let labels: Vec<String> = sizes.iter().map(|(w, h)| format!("{w} × {h}")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let selected = sizes
            .iter()
            .position(|s| *s == (mode.width, mode.height))
            .unwrap_or(0);
        let (c, all, rate) = (
            connector.clone(),
            fixed.iter().map(|m| (*m).clone()).collect::<Vec<_>>(),
            mode.refresh,
        );
        group.add(
            &self.combo("Resolution", &labels, selected, move |page, index| {
                let Some(&(w, h)) = sizes.get(index) else {
                    return;
                };
                // The same rate if this size has it, else its highest.
                let same: Vec<&Mode> = all
                    .iter()
                    .filter(|m| (m.width, m.height) == (w, h))
                    .collect();
                let pick = same
                    .iter()
                    .find(|m| (m.refresh - rate).abs() < 0.01)
                    .or_else(|| same.iter().max_by(|a, b| a.refresh.total_cmp(&b.refresh)));
                if let Some(pick) = pick {
                    let id = pick.id.clone();
                    page.edit(|d| d.set_mode(&c, &id));
                }
            }),
        );
        let rates: Vec<Mode> = fixed
            .iter()
            .filter(|m| (m.width, m.height) == (mode.width, mode.height))
            .map(|m| (*m).clone())
            .collect();
        let labels: Vec<String> = rates
            .iter()
            .map(|m| format!("{} Hz", refresh(m.refresh)))
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let selected = rates
            .iter()
            .position(|m| (m.refresh - mode.refresh).abs() < 0.01)
            .unwrap_or(0);
        let (c, variable, modes) = (connector.clone(), mode.variable, monitor.modes.clone());
        group.add(
            &self.combo("Refresh Rate", &labels, selected, move |page, index| {
                if let Some(base) = rates.get(index) {
                    // Keeps variable refresh on if this rate offers it.
                    let id = variant(&modes, base, variable).unwrap_or(&base.id).clone();
                    page.edit(|d| d.set_mode(&c, &id));
                }
            }),
        );
        if let Some(base) = fixed
            .iter()
            .find(|m| (m.width, m.height, m.refresh) == (mode.width, mode.height, mode.refresh))
        {
            if variant(&monitor.modes, base, true).is_some() {
                let row = adw::SwitchRow::builder()
                    .title("Variable Refresh Rate")
                    .subtitle(match monitor.min_refresh {
                        Some(min) => {
                            format!("Lets the rate drop to {min} Hz to match what's shown")
                        }
                        None => "Lets the rate follow what's shown".to_owned(),
                    })
                    .active(mode.variable)
                    .build();
                let (c, base, modes) = (connector.clone(), (*base).clone(), monitor.modes.clone());
                row.connect_active_notify(glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |row| {
                        let id = variant(&modes, &base, row.is_active())
                            .unwrap_or(&base.id)
                            .clone();
                        page.edit(|d| d.set_mode(&c, &id));
                    }
                ));
                group.add(&row);
            }
        }

        // Scale: the ones this mode allows.
        let labels: Vec<String> = mode
            .scales
            .iter()
            .map(|s| format!("{:.0}%", s * 100.0))
            .collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let selected = mode
            .scales
            .iter()
            .position(|s| (s - placed.scale).abs() < 1e-6)
            .unwrap_or(0);
        let (c, scales) = (connector.clone(), mode.scales.clone());
        group.add(&self.combo("Scale", &labels, selected, move |page, index| {
            if let Some(scale) = scales.get(index) {
                let scale = *scale;
                page.edit(|d| d.set_scale(&c, scale));
            }
        }));
        group
    }

    /// A drop-down that reports the chosen index; connected after the
    /// current value is set, so building it changes nothing.
    fn combo(
        &self,
        title: &str,
        labels: &[&str],
        selected: usize,
        chosen: impl Fn(&Self, usize) + 'static,
    ) -> adw::ComboRow {
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(labels))
            .build();
        row.set_selected(selected as u32);
        row.connect_selected_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |row| chosen(&page, row.selected() as usize)
        ));
        row
    }

    /// Without Mutter: the monitors sysfs lists, read-only.
    fn show_fallback(&self) {
        self.imp().state.replace(None);
        let group = adw::PreferencesGroup::builder()
            .title("Displays")
            .description("Display settings can only be changed on GNOME.")
            .build();
        for connector in display::connectors() {
            let name = connector
                .edid
                .as_ref()
                .and_then(|e| e.name.clone().or_else(|| e.text.last().cloned()))
                .unwrap_or_else(|| connector.name.clone());
            let row = adw::ActionRow::builder()
                .title(if connector.builtin() {
                    "Built-in Display".to_owned()
                } else {
                    name
                })
                .subtitle(&connector.name)
                .use_markup(false)
                .build();
            group.add(&row);
        }
        self.replace_groups(vec![group]);
    }
}

/// The monitor's name: "Built-in Display", else the EDID's model name, else
/// what Mutter calls it.
fn title(monitor: &Monitor, edid: Option<&edid::Edid>) -> String {
    if monitor.builtin {
        return "Built-in Display".to_owned();
    }
    edid.and_then(|e| e.name.clone())
        .unwrap_or_else(|| monitor.display_name.clone())
}

/// "1920 × 1080 · 144 Hz · 100% · Primary", or "Off".
fn subtitle(state: &State, monitor: &Monitor) -> String {
    let (Some(mode), Some(logical)) = (
        monitor.current_mode(),
        state.logical_for(&monitor.connector),
    ) else {
        return "Off".to_owned();
    };
    let mut parts = vec![
        format!("{} × {}", mode.width, mode.height),
        format!("{} Hz", refresh(mode.refresh)),
        format!("{:.0}%", logical.scale * 100.0),
    ];
    if logical.primary {
        parts.push("Primary".to_owned());
    }
    parts.join(" · ")
}

/// Shows each monitor's number on its own screen for two seconds. GNOME
/// Shell's own labels (`ShowMonitorLabels`) only answer GNOME Settings, so
/// these are short-lived transparent windows, one per monitor.
fn identify_displays(widget: &impl IsA<gtk::Widget>, numbers: &[(String, usize)]) {
    let monitors = widget.display().monitors();
    for monitor in monitors.iter::<gtk::gdk::Monitor>().flatten() {
        let connector = monitor
            .connector()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let Some((_, number)) = numbers.iter().find(|(c, _)| *c == connector) else {
            continue;
        };
        let label = gtk::Label::builder()
            .label(number.to_string())
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["openhub-identify-number"])
            .build();
        let window = gtk::Window::builder()
            .decorated(false)
            .css_classes(["openhub-identify"])
            .child(&label)
            .build();
        window.fullscreen_on_monitor(&monitor);
        window.present();
        glib::timeout_add_seconds_local_once(2, move || window.close());
    }
}

/// The mode with `base`'s size and rate, with variable refresh on or off.
fn variant<'m>(modes: &'m [Mode], base: &Mode, variable: bool) -> Option<&'m String> {
    modes
        .iter()
        .find(|m| {
            (m.width, m.height) == (base.width, base.height)
                && (m.refresh - base.refresh).abs() < 0.01
                && m.variable == variable
        })
        .map(|m| &m.id)
}

/// 144.0008 Hz reads as "144", 59.94 as "59.94".
pub(super) fn refresh(rate: f64) -> String {
    let rounded = (rate * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 0.01 {
        format!("{:.0}", rounded)
    } else {
        format!("{rounded:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_from_cjs_layout() {
        let path = format!(
            "{}/tests/fixtures/mutter/gnome50-laptop-and-asus.txt",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(path).unwrap();
        let ty = glib::VariantTy::new(crate::display::state::SIGNATURE).unwrap();
        let state = State::parse(&glib::Variant::parse(Some(ty), text.trim()).unwrap()).unwrap();
        assert_eq!(
            subtitle(&state, &state.monitors[0]),
            "1920 × 1080 · 144 Hz · 100% · Primary"
        );
        assert_eq!(
            subtitle(&state, &state.monitors[1]),
            "1920 × 1200 · 60 Hz · 100%"
        );
        assert_eq!(title(&state.monitors[1], None), "Built-in Display");
        assert_eq!(
            (refresh(59.9402), refresh(144.00076)),
            ("59.94".into(), "144".into())
        );
    }
}
