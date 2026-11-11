use std::cell::RefCell;
use std::collections::HashMap;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::audio::AudioManager;
use crate::bluez::BluetoothManager;
use crate::display;
use crate::fwupd;
use crate::input::InputManager;
use crate::model::{AudioDevice, Device, InputDevice, PowerDevice, UsbDevice};
use crate::shell_extension;
use crate::sysinfo::{self, About};
use crate::upower::{PowerManager, convert::kind};
use crate::usb::UsbManager;
use crate::window::OpenhubWindow;

use super::firmware_dialog;
use crate::i18n::{gettext, gettext_f, gettext_noop, ngettext_f};
use gtk::glib::clone;

/// The Connected Devices rows: (page name, title, icon).
/// In the sidebar's order.
const PAGES: [(&str, &str, &str); 7] = [
    ("power", gettext_noop("Power"), "battery-symbolic"),
    ("bluetooth", gettext_noop("Bluetooth"), "bluetooth-symbolic"),
    ("usb", gettext_noop("USB"), "media-removable-symbolic"),
    (
        "displays",
        gettext_noop("Displays"),
        "video-display-symbolic",
    ),
    (
        "keyboard",
        gettext_noop("Keyboard"),
        "input-keyboard-symbolic",
    ),
    ("mouse", gettext_noop("Mouse"), "input-mouse-symbolic"),
    ("audio", gettext_noop("Audio"), "audio-speakers-symbolic"),
];

/// A peripheral below this percentage is flagged.
const LOW_BATTERY: f64 = 20.0;

/// GNOME Settings › System pages Openhub doesn't rebuild: (title, panel).
const SETTINGS: [(&str, &str); 6] = [
    (gettext_noop("Device Name"), "system/about"),
    (gettext_noop("Region & Language"), "system/region"),
    (gettext_noop("Date & Time"), "system/datetime"),
    (gettext_noop("Users"), "system/users"),
    (gettext_noop("Remote Desktop"), "system/remote-desktop"),
    (gettext_noop("Secure Shell"), "system/secure-shell"),
];

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct SystemPage {
        pub page: adw::PreferencesPage,
        pub about: RefCell<Option<About>>,
        pub devices: adw::PreferencesGroup,
        /// Each Connected Devices card's status line and low-battery icon, by
        /// page.
        pub rows: RefCell<HashMap<&'static str, (gtk::Label, gtk::Image)>>,
        /// Mutter's last layout, for the Displays row.
        pub displays: RefCell<Option<display::State>>,
        pub subscription: RefCell<Option<gio::SignalSubscription>>,
        /// Device Firmware: hidden until fwupd answers with something.
        pub firmware: adw::PreferencesGroup,
        pub firmware_rows: RefCell<Vec<adw::ActionRow>>,
        pub fwupd_subscription: RefCell<Option<gio::SignalSubscription>>,
        pub extension_subscriptions: RefCell<Vec<gio::SignalSubscription>>,
        /// Set while the switch follows GNOME Shell, so that isn't sent back.
        pub extension_updating: std::cell::Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SystemPage {
        const NAME: &'static str = "OpenhubSystemPage";
        type Type = super::SystemPage;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for SystemPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_child(Some(&self.page));
            glib::spawn_future_local(clone!(
                #[weak]
                obj,
                async move {
                    let about = sysinfo::load().await;
                    obj.show(&about);
                    obj.imp().about.replace(Some(about));
                }
            ));
        }
    }

    impl WidgetImpl for SystemPage {}
    impl BinImpl for SystemPage {}
}

glib::wrapper! {
    /// The System page: this computer, as GNOME Settings › System › About
    /// shows it, and links to the System settings Openhub leaves to GNOME.
    pub struct SystemPage(ObjectSubclass<imp::SystemPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SystemPage {
    fn show(&self, about: &About) {
        let page = &self.imp().page;
        let windowing = sysinfo::windowing(&self.display());
        page.add(&self.header(about));
        page.add(&self.devices_group());

        let hardware = values_group(&gettext("Hardware"), &about.hardware());
        let copy = gtk::Button::builder()
            .label(gettext("Copy Details"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let report = about.report(&windowing);
        copy.connect_clicked(move |button| {
            button.clipboard().set_text(&report);
            button.set_label(&gettext("Copied"));
        });
        hardware.set_header_suffix(Some(&copy));
        page.add(&hardware);
        page.add(&values_group(
            &gettext("Software"),
            &about.software(&windowing),
        ));
        page.add(&self.firmware_group());
        page.add(&self.extension_group());

        // GNOME Settings comes with GNOME; elsewhere these rows would lead
        // nowhere.
        if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("GNOME")) {
            let group = adw::PreferencesGroup::builder()
                .title(gettext("More in GNOME Settings"))
                .build();
            for (title, panel) in SETTINGS {
                let row = adw::ActionRow::builder()
                    .title(gettext(title))
                    .use_markup(false)
                    .activatable(true)
                    .build();
                row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
                row.connect_activated(move |row| crate::ui::open_settings_panel(row, panel));
                group.add(&row);
            }
            page.add(&group);
        }
    }

    /// One row per page, with what's connected there; a row opens its page.
    fn devices_group(&self) -> adw::PreferencesGroup {
        let imp = self.imp();
        imp.devices.set_title(&gettext("Connected Devices"));
        // Cards, three to a line, fewer on a narrow window.
        let grid = gtk::FlowBox::builder()
            .homogeneous(true)
            .min_children_per_line(1)
            .max_children_per_line(3)
            .column_spacing(12)
            .row_spacing(12)
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["openhub-device-grid"])
            .build();
        let mut rows = HashMap::new();
        for (name, title, icon) in PAGES {
            let heading = gtk::Box::builder().spacing(8).build();
            heading.append(&gtk::Image::from_icon_name(icon));
            heading.append(
                &gtk::Label::builder()
                    .label(gettext(title))
                    .css_classes(["heading"])
                    .hexpand(true)
                    .xalign(0.0)
                    .build(),
            );
            let warning = gtk::Image::builder()
                .icon_name("dialog-warning-symbolic")
                .tooltip_text(gettext("Low battery"))
                .css_classes(["warning"])
                .visible(false)
                .build();
            heading.append(&warning);
            let status = gtk::Label::builder()
                .css_classes(["dim-label", "caption"])
                .wrap(true)
                .lines(3)
                // Wrapped text otherwise asks for a whole line, one card wide.
                .max_width_chars(18)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .xalign(0.0)
                .yalign(0.0)
                .vexpand(true)
                .build();
            let content = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .build();
            content.append(&heading);
            content.append(&status);
            let card = gtk::Button::builder()
                .child(&content)
                .css_classes(["card", "openhub-device-card"])
                // Translators: a card's tooltip, such as "Open Bluetooth".
                .tooltip_text(gettext_f("Open {}", &[&gettext(title)]))
                .build();
            card.connect_clicked(move |card| {
                if let Some(window) = card.root().and_downcast::<OpenhubWindow>() {
                    window.show_category(name);
                }
            });
            grid.append(&card);
            // One Tab stop per card: the button, not its grid cell.
            if let Some(cell) = card.parent() {
                cell.set_focusable(false);
            }
            rows.insert(name, (status, warning));
        }
        imp.devices.add(&grid);
        imp.rows.replace(rows);

        // USB and input lists come from sysfs scans, which the device pages
        // run every 2 s while shown; this page does the same, and that also
        // picks up connections and battery levels.
        crate::ui::rescan_while_mapped(
            self,
            clone!(
                #[weak(rename_to = page)]
                self,
                move || {
                    UsbManager::shared().refresh();
                    InputManager::mice().refresh();
                    InputManager::keyboards().refresh();
                    page.update_devices();
                }
            ),
        );
        glib::spawn_future_local(clone!(
            #[weak(rename_to = page)]
            self,
            async move { page.watch_displays().await }
        ));
        imp.devices.clone()
    }

    async fn watch_displays(&self) {
        let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
            return;
        };
        let reload = clone!(
            #[weak(rename_to = page)]
            self,
            #[strong]
            bus,
            move || {
                glib::spawn_future_local(clone!(
                    #[weak]
                    page,
                    #[strong]
                    bus,
                    async move {
                        let state = display::current_state(&bus).await.ok();
                        page.imp().displays.replace(state);
                        page.update_devices();
                    }
                ));
            }
        );
        reload();
        let subscription = display::watch(&bus, reload);
        self.imp().subscription.replace(Some(subscription));
    }

    /// Recounts everything and rewrites each row's status line.
    fn update_devices(&self) {
        let imp = self.imp();
        let rows = imp.rows.borrow();
        let set = |name: &str, status: String, low: bool| {
            if let Some((label, warning)) = rows.get(name) {
                label.set_label(&status);
                warning.set_visible(low);
            }
        };

        let bluetooth = BluetoothManager::shared();
        let paired: Vec<Device> = bluetooth.paired_devices().iter().flatten().collect();
        let connected = paired.iter().filter(|d| d.connected()).count();
        let powered = bluetooth
            .adapters()
            .iter::<crate::model::Adapter>()
            .flatten()
            .any(|a| a.powered());
        set(
            "bluetooth",
            bluetooth_status(
                bluetooth.adapters().n_items() > 0,
                powered,
                connected,
                paired.len(),
            ),
            false,
        );

        let usb = external_usb(&UsbManager::shared().devices());
        let status = match usb {
            0 => gettext("No devices"),
            n => ngettext_f("{} device", "{} devices", n as u32, &[&n.to_string()]),
        };
        set("usb", status, false);

        let power = PowerManager::shared();
        let low: Vec<PowerDevice> = power
            .peripherals()
            .iter::<PowerDevice>()
            .flatten()
            .filter(|d| d.present() && d.level() > 0.0 && d.level() < LOW_BATTERY)
            .collect();
        let mut status = if power.has_battery() {
            let status = power.status();
            if status.is_empty() {
                format!("{:.0}%", power.level())
            } else {
                format!("{:.0}% · {status}", power.level())
            }
        } else {
            match power.peripherals().n_items() {
                0 => gettext("No batteries"),
                n => ngettext_f("{} battery", "{} batteries", n, &[&n.to_string()]),
            }
        };
        for device in &low {
            status.push_str(" · ");
            // Translators: a low battery, such as "MX Master 3S at 12%".
            status.push_str(&gettext_f(
                "{} at {}",
                &[&device.title(), &device.level_text()],
            ));
        }
        set("power", status, !low.is_empty());

        for (name, manager, kind_id) in [
            ("mouse", InputManager::mice(), kind::MOUSE),
            ("keyboard", InputManager::keyboards(), kind::KEYBOARD),
        ] {
            let names: Vec<String> = manager
                .devices()
                .iter::<InputDevice>()
                .flatten()
                .map(|d| d.title())
                .collect();
            let status = if names.is_empty() {
                gettext("None connected")
            } else {
                names.join(", ")
            };
            set(name, status, low.iter().any(|d| d.kind() == kind_id));
        }

        let audio = AudioManager::shared();
        let default = |store: gio::ListStore| {
            store
                .iter::<AudioDevice>()
                .flatten()
                .find(|d| d.is_default())
                .map(|d| d.title())
        };
        let status = match (default(audio.outputs()), default(audio.inputs())) {
            (Some(output), Some(input)) => gettext_f("Output: {} · Input: {}", &[&output, &input]),
            (Some(output), None) => gettext_f("Output: {}", &[&output]),
            (None, Some(input)) => gettext_f("Input: {}", &[&input]),
            (None, None) => gettext("No audio devices"),
        };
        set("audio", status, false);

        let state = imp.displays.borrow();
        let monitors = state
            .as_ref()
            .map(|s| s.monitors.as_slice())
            .unwrap_or_default();
        let primary = state.as_ref().and_then(|state| {
            let connector = state
                .logical
                .iter()
                .find(|l| l.primary)?
                .connectors
                .first()?;
            let monitor = state.monitors.iter().find(|m| &m.connector == connector)?;
            Some(crate::ui::display::monitor_title(
                monitor,
                display::edid_for(connector).as_ref(),
            ))
        });
        let n = monitors.len();
        let count = ngettext_f("{} display", "{} displays", n as u32, &[&n.to_string()]);
        let status = match primary {
            _ if n == 0 => String::new(),
            // Translators: such as "2 displays · ASUS VP249 is primary".
            Some(primary) if n > 1 => gettext_f("{} · {} is primary", &[&count, &primary]),
            Some(primary) => primary,
            None => count,
        };
        set("displays", status, false);

        let external = monitors.iter().filter(|m| !m.builtin).count();
        imp.devices
            .set_description(Some(&headline(connected + usb + external)));
    }

    /// The GNOME Shell extension: the install command while it's missing,
    /// then a switch. Hidden where GNOME Shell's extension service isn't
    /// running.
    fn extension_group(&self) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title(gettext("GNOME Shell Extension"))
            .visible(false)
            .build();
        let row = adw::ActionRow::builder()
            .title(gettext("Openhub in the Top Bar"))
            .build();
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
        let copy = gtk::Button::builder()
            .label(gettext("Copy Command"))
            .valign(gtk::Align::Center)
            .build();
        copy.connect_clicked(|button| {
            button
                .clipboard()
                .set_text(shell_extension::INSTALL_COMMAND);
            button.set_label(&gettext("Copied"));
        });
        row.add_suffix(&copy);
        row.add_suffix(&switch);
        row.set_activatable_widget(Some(&switch));
        group.add(&row);

        glib::spawn_future_local(clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            group,
            #[weak]
            row,
            #[weak]
            switch,
            #[weak]
            copy,
            async move {
                let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await else {
                    return;
                };
                let show = clone!(
                    #[weak]
                    page,
                    #[weak]
                    group,
                    #[weak]
                    row,
                    #[weak]
                    switch,
                    #[weak]
                    copy,
                    #[strong]
                    bus,
                    move || {
                        glib::spawn_future_local(clone!(
                            #[weak]
                            page,
                            #[strong]
                            bus,
                            async move {
                                let status = shell_extension::read(&bus).await;
                                page.show_extension(&group, &row, &switch, &copy, status);
                            }
                        ));
                    }
                );
                show();
                let subscriptions = shell_extension::watch(&bus, show);
                page.imp()
                    .extension_subscriptions
                    .replace(subscriptions.into());
                switch.connect_active_notify(clone!(
                    #[weak]
                    page,
                    #[strong]
                    bus,
                    move |switch| {
                        if page.imp().extension_updating.get() {
                            return;
                        }
                        let on = switch.is_active();
                        glib::spawn_future_local(clone!(
                            #[strong]
                            bus,
                            async move {
                                if let Err(error) = shell_extension::set_enabled(&bus, on).await {
                                    glib::g_warning!(
                                        "openhub",
                                        "Can't change the extension: {error}"
                                    );
                                }
                            }
                        ));
                    }
                ));
            }
        ));
        group
    }

    fn show_extension(
        &self,
        group: &adw::PreferencesGroup,
        row: &adw::ActionRow,
        switch: &gtk::Switch,
        copy: &gtk::Button,
        status: Result<shell_extension::Status, glib::Error>,
    ) {
        use shell_extension::Status;
        // Without GNOME Shell's extension service there's nothing to show.
        let Ok(status) = status else {
            group.set_visible(false);
            return;
        };
        group.set_visible(true);
        let (subtitle, installed) = match &status {
            Status::Missing => (
                gettext_f(
                    "Not installed. Download {} from Openhub's release, run the copied command in that folder, then log out and back in.",
                    &[shell_extension::ZIP],
                ),
                false,
            ),
            Status::Installed { .. } => (
                gettext("Batteries, Bluetooth and sound output, one click away"),
                true,
            ),
            Status::Failed(error) => (
                gettext_f("The extension couldn't start: {}", &[error]),
                true,
            ),
            Status::ExtensionsOff => (
                gettext("Extensions are turned off in GNOME. Turn them on in the Extensions app."),
                true,
            ),
        };
        row.set_subtitle(&glib::markup_escape_text(&subtitle));
        copy.set_visible(!installed);
        switch.set_visible(installed);
        switch.set_sensitive(status != Status::ExtensionsOff);
        let enabled = matches!(
            status,
            Status::Installed { enabled: true } | Status::Failed(_)
        );
        self.imp().extension_updating.set(true);
        switch.set_active(enabled);
        self.imp().extension_updating.set(false);
    }

    fn firmware_group(&self) -> adw::PreferencesGroup {
        let group = &self.imp().firmware;
        group.set_title(&gettext("Device Firmware"));
        group.set_visible(false);
        glib::spawn_future_local(clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                let Ok(bus) = fwupd::bus().await else {
                    return;
                };
                page.load_firmware(&bus).await;
                // Plugged-in devices and new metadata both change the list.
                let subscription = fwupd::watch(
                    &bus,
                    clone!(
                        #[weak]
                        page,
                        #[strong]
                        bus,
                        move || {
                            glib::spawn_future_local(clone!(
                                #[weak]
                                page,
                                #[strong]
                                bus,
                                async move { page.load_firmware(&bus).await }
                            ));
                        }
                    ),
                );
                page.imp().fwupd_subscription.replace(Some(subscription));
            }
        ));
        group.clone()
    }

    /// Reads fwupd's devices and their updates, then rebuilds the rows.
    async fn load_firmware(&self, bus: &gio::DBusConnection) {
        // Without fwupd the group stays hidden, with no error.
        let Some(devices) = fwupd::devices(bus).await else {
            return;
        };
        let mut updates = HashMap::new();
        for device in devices.iter().filter(|d| d.updatable) {
            if let Some(release) = fwupd::upgrade(bus, &device.id).await {
                updates.insert(device.id.clone(), release);
            }
        }
        let mut shown = fwupd::shown(&devices, |d| updates.contains_key(&d.id));
        // Updates first, then by name.
        shown.sort_by_key(|d| (!updates.contains_key(&d.id), d.name.to_lowercase()));

        let imp = self.imp();
        for row in imp.firmware_rows.take() {
            imp.firmware.remove(&row);
        }
        let count = updates.len();
        imp.firmware.set_description(Some(&match count {
            0 => gettext("Everything is up to date."),
            n => ngettext_f(
                "{} update available",
                "{} updates available",
                n as u32,
                &[&n.to_string()],
            ),
        }));
        let mut rows = Vec::new();
        for device in shown {
            let update = updates.get(&device.id).cloned();
            let subtitle = [device.vendor.as_str(), device.version.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            let row = adw::ActionRow::builder()
                .title(&device.name)
                .subtitle(subtitle)
                .use_markup(false)
                .activatable(true)
                .build();
            if update.is_some() {
                row.add_suffix(
                    &gtk::Label::builder()
                        .label(gettext("Update Available"))
                        .valign(gtk::Align::Center)
                        .css_classes(["accent", "caption-heading"])
                        .build(),
                );
            }
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            let device = device.clone();
            row.connect_activated(move |row| {
                firmware_dialog::new(&device, update.as_ref()).present(Some(row));
            });
            imp.firmware.add(&row);
            rows.push(row);
        }
        imp.firmware.set_visible(!rows.is_empty());
        imp.firmware_rows.replace(rows);
    }

    /// The distro logo (or the computer's icon), its name and model.
    fn header(&self, about: &About) -> adw::PreferencesGroup {
        let theme = gtk::IconTheme::for_display(&self.display());
        let icon = about
            .logo
            .clone()
            .filter(|logo| theme.has_icon(logo))
            .unwrap_or_else(|| about.icon.clone());
        let image = gtk::Image::builder()
            .icon_name(icon)
            .pixel_size(128)
            .margin_bottom(12)
            .build();
        let name = gtk::Label::builder()
            .label(&about.name)
            .css_classes(["title-1"])
            .wrap(true)
            .selectable(true)
            .build();
        let model = gtk::Label::builder()
            .label(&about.model)
            .css_classes(["dim-label"])
            .wrap(true)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        content.append(&image);
        content.append(&name);
        content.append(&model);
        let group = adw::PreferencesGroup::new();
        group.add(&content);
        group
    }
}

/// Devices plugged in over USB, not counting hubs or built-in ones.
fn external_usb(devices: &gio::ListStore) -> usize {
    devices
        .iter::<UsbDevice>()
        .flatten()
        .map(|d| {
            let own = usize::from(!d.hub() && !d.internal());
            own + external_usb(&d.children())
        })
        .sum()
}

fn bluetooth_status(adapter: bool, powered: bool, connected: usize, paired: usize) -> String {
    match (adapter, powered, connected) {
        (false, _, _) => gettext("No adapter"),
        (true, false, _) => gettext("Off"),
        (true, true, 0) if paired == 0 => gettext("No devices"),
        (true, true, 0) => ngettext_f(
            "{} paired",
            "{} paired",
            paired as u32,
            &[&paired.to_string()],
        ),
        (true, true, c) => ngettext_f(
            "{} connected, {} paired",
            "{} connected, {} paired",
            c as u32,
            &[&c.to_string(), &paired.to_string()],
        ),
    }
}

/// Each physical device once: a Bluetooth mouse is on the Bluetooth, Mouse
/// and Power rows, but here it's one of the connected Bluetooth devices.
/// USB and external displays are the rest.
fn headline(devices: usize) -> String {
    match devices {
        0 => gettext("No devices connected"),
        n => ngettext_f(
            "{} device connected",
            "{} devices connected",
            n as u32,
            &[&n.to_string()],
        ),
    }
}

/// A group of rows with each value on the right, selectable for copying.
fn values_group(title: &str, rows: &[(&str, String)]) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    for (name, value) in rows {
        let row = adw::ActionRow::builder()
            .title(gettext(*name))
            .use_markup(false)
            .build();
        let label = gtk::Label::builder()
            .label(value)
            .css_classes(["dim-label"])
            .selectable(true)
            .wrap(true)
            .hexpand(true)
            .xalign(1.0)
            .justify(gtk::Justification::Right)
            .build();
        row.add_suffix(&label);
        group.add(&row);
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_status_lines() {
        assert_eq!(bluetooth_status(false, false, 0, 0), "No adapter");
        assert_eq!(bluetooth_status(true, false, 0, 5), "Off");
        assert_eq!(bluetooth_status(true, true, 0, 5), "5 paired");
        assert_eq!(bluetooth_status(true, true, 2, 5), "2 connected, 5 paired");
        assert_eq!(headline(9), "9 devices connected");
    }
}
