use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use super::canvas::{Canvas, Item};
use super::details_dialog;
use crate::ddc;
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
        /// Mutter's backlights and the serial `SetBacklight` wants.
        pub backlights: RefCell<Option<(u32, Vec<display::Backlight>)>>,
        /// External monitors' DDC/CI brightness: (current, maximum).
        pub ddc: RefCell<HashMap<String, (u16, u16)>>,
        pub backlight_subscription: RefCell<Option<gio::SignalSubscription>>,
        pub shell_subscription: RefCell<Option<gio::SignalSubscription>>,
        /// A burst of GNOME Shell's signals is followed once.
        pub follow_queued: Cell<bool>,
        /// Each monitor's brightness slider, updated in place when the
        /// brightness changes elsewhere.
        pub sliders: RefCell<HashMap<String, gtk::Scale>>,
        /// Brightness values waiting to be sent, at most one write per
        /// monitor every 200 ms while a slider moves.
        pub pending: RefCell<HashMap<String, f64>>,
        pub writing: Cell<bool>,
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
        // Brightness keys and Quick Settings change it too.
        let backlights = display::watch_backlights(
            &bus,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    glib::spawn_future_local(glib::clone!(
                        #[weak]
                        page,
                        async move { page.reload_backlights().await }
                    ));
                }
            ),
        );
        self.imp().backlight_subscription.replace(Some(backlights));
        // The brightness keys and Quick Settings go through GNOME Shell,
        // which sends a burst of these while a key is held.
        let shell = display::watch_shell_brightness(
            &bus,
            glib::clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    if !page.imp().follow_queued.replace(true) {
                        glib::idle_add_local_once(glib::clone!(
                            #[weak]
                            page,
                            move || {
                                page.imp().follow_queued.set(false);
                                page.follow_kernel_brightness();
                            }
                        ));
                    }
                }
            ),
        );
        self.imp().shell_subscription.replace(Some(shell));
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
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[strong]
            state,
            async move { page.load_brightness(&state).await }
        ));
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
        if let Some(group) = self.night_light_group() {
            groups.push(group);
        }
        self.replace_groups(groups);
    }

    /// Reads Mutter's backlights, and external monitors' brightness over
    /// DDC/CI once (many answer slowly), then redraws.
    async fn load_brightness(&self, state: &State) {
        self.reload_backlights().await;
        let mutter: Vec<String> = self
            .imp()
            .backlights
            .borrow()
            .iter()
            .flat_map(|(_, b)| b.iter().map(|b| b.connector.clone()))
            .collect();
        let external: Vec<String> = state
            .monitors
            .iter()
            .filter(|m| !m.builtin && !mutter.contains(&m.connector))
            .map(|m| m.connector.clone())
            .collect();
        let read = gio::spawn_blocking(move || {
            external
                .into_iter()
                .filter_map(|connector| {
                    let bus = ddc::bus_for(&connector)?;
                    let value = ddc::Monitor::open(&bus).ok()?.get(ddc::BRIGHTNESS).ok()?;
                    Some((connector, value))
                })
                .collect::<HashMap<_, _>>()
        })
        .await
        .unwrap_or_default();
        self.imp().ddc.replace(read);
        self.render();
    }

    /// Moves Mutter-controlled sliders to the kernel's real level, which the
    /// brightness keys change without updating Mutter's property.
    fn follow_kernel_brightness(&self) {
        let imp = self.imp();
        if imp.writing.get() {
            return;
        }
        let mutter = imp.backlights.borrow();
        for (connector, slider) in imp.sliders.borrow().iter() {
            let controlled = mutter
                .as_ref()
                .is_some_and(|(_, list)| list.iter().any(|b| &b.connector == connector));
            if let (true, Some(level)) = (controlled, display::kernel_backlight(connector)) {
                slider.set_value(f64::from(level));
            }
        }
    }

    /// Mutter's backlights again, moving sliders that aren't being dragged.
    async fn reload_backlights(&self) {
        let Some(bus) = self.imp().bus.borrow().clone() else {
            return;
        };
        let Ok(backlights) = display::backlights(&bus).await else {
            return;
        };
        let imp = self.imp();
        if !imp.writing.get() {
            for backlight in &backlights.1 {
                if let Some(slider) = imp.sliders.borrow().get(&backlight.connector) {
                    slider.set_value(f64::from(backlight.value));
                }
            }
        }
        imp.backlights.replace(Some(backlights));
    }

    /// The monitor's brightness row: through Mutter where it has a backlight,
    /// else over DDC/CI where the monitor answered.
    fn brightness_row(&self, connector: &str) -> Option<adw::ActionRow> {
        let imp = self.imp();
        let (low, high, value) = match imp
            .backlights
            .borrow()
            .as_ref()
            .and_then(|(_, list)| list.iter().find(|b| b.connector == connector).cloned())
        {
            // The kernel's level: Mutter's can be stale after the keys.
            Some(b) => {
                let level = display::kernel_backlight(connector).unwrap_or(b.value);
                (f64::from(b.min), f64::from(b.max), f64::from(level))
            }
            None => {
                let (current, max) = *imp.ddc.borrow().get(connector)?;
                (0.0, f64::from(max), f64::from(current))
            }
        };
        let slider = gtk::Scale::with_range(gtk::Orientation::Horizontal, low, high, 1.0);
        slider.set_value(value);
        slider.set_width_request(240);
        slider.set_valign(gtk::Align::Center);
        slider.update_property(&[gtk::accessible::Property::Label("Brightness")]);
        let c = connector.to_owned();
        // Only the user's moves are sent; updates from elsewhere aren't.
        slider.connect_change_value(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, _, value| {
                page.imp().pending.borrow_mut().insert(c.clone(), value);
                if !page.imp().writing.get() {
                    page.flush_brightness();
                }
                glib::Propagation::Proceed
            }
        ));
        imp.sliders
            .borrow_mut()
            .insert(connector.to_owned(), slider.clone());
        let row = adw::ActionRow::builder().title("Brightness").build();
        row.add_suffix(&slider);
        Some(row)
    }

    /// Sends the newest brightness for each monitor, then waits 200 ms
    /// before sending again, so a dragged slider doesn't flood the bus.
    fn flush_brightness(&self) {
        let imp = self.imp();
        let pending = imp.pending.take();
        if pending.is_empty() {
            imp.writing.set(false);
            return;
        }
        imp.writing.set(true);
        let mutter = imp.backlights.borrow().clone();
        let bus = imp.bus.borrow().clone();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                for (connector, value) in pending {
                    let through_mutter = mutter
                        .as_ref()
                        .filter(|(_, list)| list.iter().any(|b| b.connector == connector));
                    match (through_mutter, &bus) {
                        (Some((serial, _)), Some(bus)) => {
                            let _ = display::set_backlight(
                                bus,
                                *serial,
                                &connector,
                                value.round() as i32,
                            )
                            .await;
                        }
                        _ => {
                            let level = value.round().clamp(0.0, f64::from(u16::MAX)) as u16;
                            let _ = gio::spawn_blocking(move || {
                                let bus = ddc::bus_for(&connector)?;
                                ddc::Monitor::open(&bus)
                                    .ok()?
                                    .set(ddc::BRIGHTNESS, level)
                                    .ok()
                            })
                            .await;
                        }
                    }
                }
                glib::timeout_future(std::time::Duration::from_millis(200)).await;
                page.flush_brightness();
            }
        ));
    }

    /// Night Light, through the same GNOME settings Quick Settings uses.
    /// Shown only where that schema is installed.
    fn night_light_group(&self) -> Option<adw::PreferencesGroup> {
        const SCHEMA: &str = "org.gnome.settings-daemon.plugins.color";
        gio::SettingsSchemaSource::default()?.lookup(SCHEMA, true)?;
        let settings = gio::Settings::new(SCHEMA);
        let group = adw::PreferencesGroup::builder()
            .title("Night Light")
            .build();
        let enabled = adw::SwitchRow::builder()
            .title("Night Light")
            .subtitle("Makes the screen warmer, which can help you sleep")
            .build();
        settings
            .bind("night-light-enabled", &enabled, "active")
            .build();
        group.add(&enabled);

        let automatic = settings.boolean("night-light-schedule-automatic");
        let schedule = adw::ComboRow::builder()
            .title("Schedule")
            .model(&gtk::StringList::new(&[
                "Sunset to Sunrise",
                "Manual Schedule",
            ]))
            .selected(u32::from(!automatic))
            .build();
        group.add(&schedule);
        let from = time_row("From", &settings, "night-light-schedule-from");
        let to = time_row("To", &settings, "night-light-schedule-to");
        for row in [&from, &to] {
            row.set_visible(!automatic);
            group.add(row);
        }
        schedule.connect_selected_notify(glib::clone!(
            #[strong]
            settings,
            #[weak]
            from,
            #[weak]
            to,
            move |row| {
                let manual = row.selected() == 1;
                let _ = settings.set_boolean("night-light-schedule-automatic", !manual);
                from.set_visible(manual);
                to.set_visible(manual);
            }
        ));

        // GNOME Settings' range: warmest on the left.
        let temperature =
            gtk::Scale::with_range(gtk::Orientation::Horizontal, 1700.0, 4700.0, 100.0);
        temperature.set_value(f64::from(settings.uint("night-light-temperature")));
        temperature.set_width_request(240);
        temperature.set_valign(gtk::Align::Center);
        temperature.update_property(&[gtk::accessible::Property::Label("Color Temperature")]);
        temperature.connect_change_value(glib::clone!(
            #[strong]
            settings,
            move |_, _, value| {
                let kelvin = value.round().clamp(1700.0, 4700.0) as u32;
                let _ = settings.set_uint("night-light-temperature", kelvin);
                glib::Propagation::Proceed
            }
        ));
        let row = adw::ActionRow::builder()
            .title("Color Temperature")
            .subtitle("Warmer to the left")
            .build();
        row.add_suffix(&temperature);
        group.add(&row);
        enabled
            .bind_property("active", &schedule, "sensitive")
            .sync_create()
            .build();
        Some(group)
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
        if let Some(row) = self.brightness_row(&connector) {
            group.add(&row);
        }
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

/// A time of day for Night Light's manual schedule: hours and minutes,
/// stored as fractional hours (20.5 is 20:30), as GNOME does.
fn time_row(title: &str, settings: &gio::Settings, key: &'static str) -> adw::ActionRow {
    let value = settings.double(key);
    let hours = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let minutes = gtk::SpinButton::with_range(0.0, 59.0, 1.0);
    hours.set_value(value.trunc());
    minutes.set_value(((value.fract() * 60.0).round()).min(59.0));
    for (spin, label) in [(&hours, "Hours"), (&minutes, "Minutes")] {
        spin.set_valign(gtk::Align::Center);
        spin.set_orientation(gtk::Orientation::Vertical);
        spin.update_property(&[gtk::accessible::Property::Label(&format!(
            "{title} {label}"
        ))]);
    }
    let save = glib::clone!(
        #[strong]
        settings,
        #[weak]
        hours,
        #[weak]
        minutes,
        move || {
            let _ = settings.set_double(key, hours.value() + minutes.value() / 60.0);
        }
    );
    hours.connect_value_changed(glib::clone!(
        #[strong]
        save,
        move |_| save()
    ));
    minutes.connect_value_changed(move |_| save());
    let row = adw::ActionRow::builder().title(title).build();
    let colon = gtk::Label::new(Some(":"));
    let time = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    time.append(&hours);
    time.append(&colon);
    time.append(&minutes);
    row.add_suffix(&time);
    row
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
