use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::model::InputDevice;
use crate::via::definition::{Control, ControlKind, shows};
use crate::via::{self, Definition, Device, Node, keycodes};

/// Key size on the layout, in pixels per key unit.
const UNIT: f64 = 40.0;

/// A keyboard's lighting and keymap over VIA, from its definition file, and
/// a way to its shortcuts in GNOME Settings.
pub fn new(device: &InputDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .content_width(720)
        .content_height(560)
        .build();
    let page = adw::PreferencesPage::new();
    dialog.add(&page);
    let usb_device = device.usb_device();
    let node = via::find()
        .into_iter()
        .find(|node| !usb_device.is_empty() && node.usb_device == usb_device);
    // Shortcuts always come last, so each path below adds them after its own groups.
    let shortcuts = shortcuts_group();
    match node {
        None => page.add(&note(if device.connection() == "Bluetooth" {
            "Connect this keyboard with its USB cable to change its lighting and keys. \
             VIA only works over the cable."
        } else {
            "Openhub can't change this keyboard's lighting or keys. It works with VIA \
             keyboards connected by cable."
        })),
        Some(node) => match via::load_definition(node.vendor, node.product) {
            Some(definition) => {
                load(&dialog, &page, node, definition, shortcuts);
                return dialog;
            }
            None => page.add(&import_group(&dialog, &page, node, &shortcuts)),
        },
    }
    page.add(&shortcuts);
    dialog
}

fn note(text: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().description(text).build()
}

/// For a VIA keyboard Openhub has no definition for yet.
fn import_group(
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    node: Node,
    shortcuts: &adw::PreferencesGroup,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Lighting and Keys")
        .description(format!(
            "This is a VIA keyboard ({:04X}:{:04X}). Import its VIA definition, the JSON file \
             the keyboard's maker provides for the VIA app, to change its lighting and keys.",
            node.vendor, node.product
        ))
        .build();
    let button = gtk::Button::builder()
        .label("Import…")
        .valign(gtk::Align::Center)
        .build();
    let row = adw::ActionRow::builder().title("VIA Definition").build();
    row.add_suffix(&button);
    group.add(&row);
    button.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        page,
        #[weak]
        group,
        #[weak]
        shortcuts,
        move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("VIA definitions"));
            filter.add_suffix("json");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let chooser = gtk::FileDialog::builder()
                .title("Import VIA Definition")
                .filters(&filters)
                .build();
            let parent = dialog.root().and_downcast::<gtk::Window>();
            let node = node.clone();
            chooser.open(parent.as_ref(), gio::Cancellable::NONE, move |file| {
                let Ok(file) = file else {
                    return; // Cancelled.
                };
                file.load_contents_async(gio::Cancellable::NONE, move |result| {
                    let imported = result.map_err(|e| e.to_string()).and_then(|(bytes, _)| {
                        via::import_definition(
                            node.vendor,
                            node.product,
                            &String::from_utf8_lossy(&bytes),
                        )
                    });
                    match imported {
                        Ok(definition) => {
                            page.remove(&group);
                            page.remove(&shortcuts);
                            load(&dialog, &page, node, definition, shortcuts);
                        }
                        Err(error) => toast(&dialog, &error),
                    }
                });
            });
        }
    ));
    group
}

/// What a read of the keyboard returns.
struct Snapshot {
    values: HashMap<(u8, u8), Vec<u8>>,
    keycodes: Vec<u16>,
}

fn read(node: &Node, definition: &Definition) -> std::io::Result<Snapshot> {
    let mut device = Device::open(node)?;
    let mut values = HashMap::new();
    for control in &definition.controls {
        let value = device.value(control.channel, control.value_id, control.kind.len())?;
        values.insert((control.channel, control.value_id), value);
    }
    let keycodes = definition
        .keys
        .iter()
        .map(|key| device.keycode(0, key.row, key.col))
        .collect::<std::io::Result<_>>()?;
    Ok(Snapshot { values, keycodes })
}

/// Reads the keyboard, then fills the page above the shortcuts group.
fn load(
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    node: Node,
    definition: Definition,
    shortcuts: adw::PreferencesGroup,
) {
    let loading = adw::PreferencesGroup::new();
    loading.add(
        &gtk::Spinner::builder()
            .spinning(true)
            .height_request(32)
            .margin_top(24)
            .margin_bottom(24)
            .build(),
    );
    page.add(&loading);
    glib::spawn_future_local(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        page,
        async move {
            let (read_node, read_definition) = (node.clone(), definition.clone());
            let snapshot = gio::spawn_blocking(move || read(&read_node, &read_definition)).await;
            page.remove(&loading);
            let snapshot = match snapshot {
                Ok(Ok(snapshot)) => snapshot,
                Ok(Err(error)) => {
                    page.add(&note(&format!("Couldn't read the keyboard: {error}")));
                    page.add(&shortcuts);
                    return;
                }
                Err(_) => {
                    page.add(&note("Couldn't read the keyboard"));
                    page.add(&shortcuts);
                    return;
                }
            };
            let editor = Rc::new(Editor {
                node,
                definition,
                values: RefCell::new(snapshot.values),
                rows: RefCell::default(),
                pending: RefCell::default(),
                write_queued: Cell::new(false),
                dialog: dialog.downgrade(),
            });
            for group in editor.lighting_groups() {
                page.add(&group);
            }
            page.add(&editor.keys_group(snapshot.keycodes));
            page.add(&shortcuts);
            editor.sync_visibility();
            // Lighting changes apply live; this keeps them after unplugging.
            dialog.connect_closed(move |_| editor.save());
        }
    ));
}

struct Editor {
    node: Node,
    definition: Definition,
    /// Current value bytes by (channel, value ID).
    values: RefCell<HashMap<(u8, u8), Vec<u8>>>,
    /// Each control's row, for `showIf`.
    rows: RefCell<Vec<(Control, gtk::Widget)>>,
    /// Lighting writes not sent yet, coalesced while a slider moves.
    pending: RefCell<HashMap<(u8, u8), Vec<u8>>>,
    write_queued: Cell<bool>,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
}

impl Editor {
    fn lighting_groups(self: &Rc<Self>) -> Vec<adw::PreferencesGroup> {
        let mut groups: Vec<(String, adw::PreferencesGroup)> = Vec::new();
        for control in &self.definition.controls {
            if groups
                .last()
                .is_none_or(|(title, _)| *title != control.section)
            {
                let group = adw::PreferencesGroup::builder()
                    .title(&control.section)
                    .build();
                groups.push((control.section.clone(), group));
            }
            let row = self.control_row(control);
            groups.last().unwrap().1.add(&row);
            self.rows.borrow_mut().push((control.clone(), row));
        }
        groups.into_iter().map(|(_, group)| group).collect()
    }

    fn control_row(self: &Rc<Self>, control: &Control) -> gtk::Widget {
        let key = (control.channel, control.value_id);
        let current = self.values.borrow().get(&key).cloned().unwrap_or_default();
        let first = current.first().copied().unwrap_or_default();
        match &control.kind {
            ControlKind::Range(low, high) => {
                let scale = gtk::Scale::with_range(
                    gtk::Orientation::Horizontal,
                    f64::from(*low),
                    f64::from(*high),
                    1.0,
                );
                scale.set_value(f64::from(first));
                scale.set_width_request(240);
                scale.set_valign(gtk::Align::Center);
                scale.update_property(&[gtk::accessible::Property::Label(&control.label)]);
                scale.connect_value_changed(glib::clone!(
                    #[weak(rename_to = editor)]
                    self,
                    move |scale| editor.change(key, vec![scale.value() as u8])
                ));
                let row = adw::ActionRow::builder().title(&control.label).build();
                row.add_suffix(&scale);
                row.upcast()
            }
            ControlKind::Dropdown(options) => {
                let labels: Vec<&str> = options.iter().map(|(label, _)| label.as_str()).collect();
                let row = adw::ComboRow::builder()
                    .title(&control.label)
                    .model(&gtk::StringList::new(&labels))
                    .build();
                if let Some(index) = options.iter().position(|(_, value)| *value == first) {
                    row.set_selected(index as u32);
                }
                let values: Vec<u8> = options.iter().map(|(_, value)| *value).collect();
                row.connect_selected_notify(glib::clone!(
                    #[weak(rename_to = editor)]
                    self,
                    move |row| {
                        if let Some(value) = values.get(row.selected() as usize) {
                            editor.change(key, vec![*value]);
                        }
                    }
                ));
                row.upcast()
            }
            ControlKind::Toggle => {
                let row = adw::SwitchRow::builder()
                    .title(&control.label)
                    .active(first != 0)
                    .build();
                row.connect_active_notify(glib::clone!(
                    #[weak(rename_to = editor)]
                    self,
                    move |row| editor.change(key, vec![u8::from(row.is_active())])
                ));
                row.upcast()
            }
            ControlKind::Color => {
                // Hue and saturation as 0 to 255; brightness is its own control.
                let (hue, saturation) = (first, current.get(1).copied().unwrap_or(255));
                let (r, g, b) =
                    gtk::hsv_to_rgb(f32::from(hue) / 255.0, f32::from(saturation) / 255.0, 1.0);
                let button = gtk::ColorDialogButton::builder()
                    .dialog(&gtk::ColorDialog::builder().with_alpha(false).build())
                    .rgba(&gtk::gdk::RGBA::new(r, g, b, 1.0))
                    .valign(gtk::Align::Center)
                    .build();
                button.connect_rgba_notify(glib::clone!(
                    #[weak(rename_to = editor)]
                    self,
                    move |button| {
                        let rgba = button.rgba();
                        let (h, s, _) = gtk::rgb_to_hsv(rgba.red(), rgba.green(), rgba.blue());
                        editor.change(
                            key,
                            vec![(h * 255.0).round() as u8, (s * 255.0).round() as u8],
                        );
                    }
                ));
                let row = adw::ActionRow::builder().title(&control.label).build();
                row.add_suffix(&button);
                row.upcast()
            }
        }
    }

    /// Shows each control whose `showIf` holds for the current values.
    fn sync_visibility(&self) {
        let by_id: HashMap<String, u8> = {
            let values = self.values.borrow();
            self.rows
                .borrow()
                .iter()
                .filter_map(|(control, _)| {
                    let value = values.get(&(control.channel, control.value_id))?.first()?;
                    Some((control.id.clone(), *value))
                })
                .collect()
        };
        for (control, row) in self.rows.borrow().iter() {
            row.set_visible(control.show_if.as_deref().is_none_or(|c| shows(c, &by_id)));
        }
    }

    /// Applies a lighting value live, sending at most one write per burst.
    fn change(self: &Rc<Self>, key: (u8, u8), value: Vec<u8>) {
        self.values.borrow_mut().insert(key, value.clone());
        self.pending.borrow_mut().insert(key, value);
        self.sync_visibility();
        if self.write_queued.replace(true) {
            return;
        }
        let editor = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(50), move || {
            editor.write_queued.set(false);
            let writes: Vec<((u8, u8), Vec<u8>)> = editor.pending.take().into_iter().collect();
            let node = editor.node.clone();
            glib::spawn_future_local(async move {
                let written = gio::spawn_blocking(move || {
                    let mut device = Device::open(&node)?;
                    for ((channel, id), value) in &writes {
                        device.set_value(*channel, *id, value)?;
                    }
                    std::io::Result::Ok(())
                })
                .await;
                if let Ok(Err(error)) = written {
                    editor.toast(&format!("Couldn't change the lighting: {error}"));
                }
            });
        });
    }

    /// Stores the lighting on the keyboard, once per channel used.
    fn save(&self) {
        let mut channels: Vec<u8> = self.definition.controls.iter().map(|c| c.channel).collect();
        channels.sort_unstable();
        channels.dedup();
        let node = self.node.clone();
        // Fire and forget: the dialog is closing.
        gio::spawn_blocking(move || {
            if let Ok(mut device) = Device::open(&node) {
                for channel in channels {
                    let _ = device.save(channel);
                }
            }
        });
    }

    /// The layout with each key's current layer-0 assignment; clicking a
    /// key picks a new one, written to the keyboard at once.
    fn keys_group(self: &Rc<Self>, codes: Vec<u16>) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::builder()
            .title("Keys")
            .description("Click a key to choose what it does. Changes are stored on the keyboard.")
            .build();
        let layout = gtk::Fixed::new();
        layout.set_halign(gtk::Align::Center);
        layout.set_margin_top(6);
        layout.set_margin_bottom(6);
        for (key, code) in self.definition.keys.iter().zip(codes) {
            // Long names wrap to two lines rather than widen the key past
            // its place on the layout.
            let label = gtk::Label::builder()
                .label(keycodes::name(code, &self.definition.custom_keycodes))
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .max_width_chars(1)
                .justify(gtk::Justification::Center)
                .css_classes(["caption"])
                .build();
            let button = gtk::Button::builder()
                .child(&label)
                .css_classes(["via-key"])
                .width_request((key.w * UNIT) as i32 - 4)
                .height_request((key.h * UNIT) as i32 - 4)
                .build();
            button.update_property(&[gtk::accessible::Property::Label(&format!(
                "Row {} key {}: {}",
                key.row + 1,
                key.col + 1,
                label.label()
            ))]);
            let (row, col) = (key.row, key.col);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = editor)]
                self,
                #[weak]
                label,
                move |button| editor.pick(button, &label, row, col)
            ));
            layout.put(&button, key.x * UNIT, key.y * UNIT);
        }
        let card = gtk::Box::builder().css_classes(["card"]).build();
        card.append(&layout);
        layout.set_hexpand(true);
        group.add(&card);

        let reset = gtk::Button::builder()
            .label("Restore Default Keys")
            .halign(gtk::Align::Center)
            .margin_top(12)
            .css_classes(["pill"])
            .build();
        reset.connect_clicked(glib::clone!(
            #[weak(rename_to = editor)]
            self,
            move |_| editor.confirm_reset()
        ));
        group.add(&reset);
        group
    }

    /// A popover of keys; the choice is written to the keyboard.
    fn pick(self: &Rc<Self>, anchor: &gtk::Button, label: &gtk::Label, row: u8, col: u8) {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let popover = gtk::Popover::builder().build();
        let mut groups = keycodes::groups();
        let custom: Vec<(u16, String)> = self
            .definition
            .custom_keycodes
            .iter()
            .enumerate()
            .map(|(index, name)| (0x7E00 + index as u16, name.clone()))
            .collect();
        if !custom.is_empty() {
            groups.push(("This Keyboard", custom));
        }
        for (title, keys) in groups {
            content.append(
                &gtk::Label::builder()
                    .label(title)
                    .xalign(0.0)
                    .css_classes(["heading"])
                    .build(),
            );
            let flow = gtk::FlowBox::builder()
                .selection_mode(gtk::SelectionMode::None)
                .max_children_per_line(8)
                .homogeneous(true)
                .build();
            for (code, name) in keys {
                let button = gtk::Button::builder().label(&name).build();
                button.connect_clicked(glib::clone!(
                    #[weak(rename_to = editor)]
                    self,
                    #[weak]
                    popover,
                    #[weak]
                    label,
                    move |_| {
                        popover.popdown();
                        editor.assign(&label, row, col, code);
                    }
                ));
                flow.insert(&button, -1);
            }
            content.append(&flow);
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_width(560)
            .min_content_height(360)
            .build();
        popover.set_child(Some(&scrolled));
        popover.set_parent(anchor);
        popover.connect_closed(|popover| popover.unparent());
        popover.popup();
    }

    fn assign(self: &Rc<Self>, label: &gtk::Label, row: u8, col: u8, code: u16) {
        let node = self.node.clone();
        let editor = self.clone();
        let label = label.clone();
        glib::spawn_future_local(async move {
            let written =
                gio::spawn_blocking(move || Device::open(&node)?.set_keycode(0, row, col, code))
                    .await;
            match written {
                Ok(Ok(())) => {
                    label.set_label(&keycodes::name(code, &editor.definition.custom_keycodes))
                }
                Ok(Err(error)) => editor.toast(&format!("Couldn't change the key: {error}")),
                Err(_) => editor.toast("Couldn't change the key"),
            }
        });
    }

    fn confirm_reset(self: &Rc<Self>) {
        let Some(dialog) = self.dialog.upgrade() else {
            return;
        };
        let confirm = adw::AlertDialog::new(
            Some("Restore Default Keys?"),
            Some("Every key goes back to what the keyboard's firmware sets, on all layers."),
        );
        confirm.add_responses(&[("cancel", "Cancel"), ("reset", "Restore")]);
        confirm.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
        confirm.set_default_response(Some("cancel"));
        let editor = self.clone();
        confirm.connect_response(None, move |_, response| {
            if response != "reset" {
                return;
            }
            let node = editor.node.clone();
            let definition = editor.definition.clone();
            let editor = editor.clone();
            glib::spawn_future_local(async move {
                let result = gio::spawn_blocking(move || {
                    Device::open(&node)?.reset_keymap()?;
                    read(&node, &definition)
                })
                .await;
                match result {
                    Ok(Ok(_)) => editor.toast("Keys restored. Reopen this window to see them."),
                    Ok(Err(error)) => editor.toast(&format!("Couldn't restore the keys: {error}")),
                    Err(_) => editor.toast("Couldn't restore the keys"),
                }
            });
        });
        confirm.present(Some(&dialog));
    }

    fn toast(&self, title: &str) {
        if let Some(dialog) = self.dialog.upgrade() {
            toast(&dialog, title);
        }
    }
}

fn toast(dialog: &adw::PreferencesDialog, title: &str) {
    dialog.add_toast(adw::Toast::builder().title(title).use_markup(false).build());
}

/// Every keyboard: its shortcuts are a GNOME setting, not a device one.
fn shortcuts_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let row = adw::ActionRow::builder()
        .title("Keyboard Shortcuts")
        .subtitle("Opens GNOME Settings")
        .activatable(true)
        .build();
    row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
    row.connect_activated(|row| {
        // Wayland only lets GNOME Settings come to the front with a token
        // from the app that asked.
        let token = row
            .display()
            .app_launch_context()
            .startup_notify_id(None::<&gio::AppInfo>, &[]);
        glib::spawn_future_local(async move {
            if let Err(error) = open_keyboard_settings(token.as_deref()).await {
                glib::g_warning!("openhub", "Can't open GNOME Settings: {error}");
            }
        });
    });
    group.add(&row);
    group
}

/// GNOME Settings' Keyboard panel, through its `launch-panel` action.
async fn open_keyboard_settings(token: Option<&str>) -> Result<(), glib::Error> {
    let bus = gio::bus_get_future(gio::BusType::Session).await?;
    let panel = ("keyboard", Vec::<glib::Variant>::new()).to_variant();
    let args = (
        "launch-panel",
        vec![panel],
        token
            .map(|token| {
                HashMap::from([
                    ("activation-token".to_owned(), token.to_variant()),
                    ("desktop-startup-id".to_owned(), token.to_variant()),
                ])
            })
            .unwrap_or_default(),
    )
        .to_variant();
    bus.call_future(
        Some("org.gnome.Settings"),
        "/org/gnome/Settings",
        "org.gtk.Actions",
        "Activate",
        Some(&args),
        None,
        gio::DBusCallFlags::NONE,
        5000,
    )
    .await
    .map(drop)
}
