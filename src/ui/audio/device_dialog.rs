use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::audio::AudioManager;
use crate::eq::{self, Settings};
use crate::model::AudioDevice;

/// An output's or input's details, with the equalizer below for outputs.
/// Details show the values at the time it opens; EQ changes apply at once
/// and are saved.
pub fn new(manager: &AudioManager, device: &AudioDevice) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::builder()
        .title(device.title())
        .content_width(560)
        .build();
    let page = adw::PreferencesPage::new();
    page.add(&details(manager, device));
    dialog.add(&page);
    if device.output() && manager.equalizer() {
        equalizer(&dialog, &page, manager, device);
    }
    dialog
}

fn details(manager: &AudioManager, device: &AudioDevice) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    let default = adw::ActionRow::builder()
        .title(if device.output() {
            "Default Output"
        } else {
            "Default Input"
        })
        .build();
    let check = gtk::Image::from_icon_name("object-select-symbolic");
    check.update_property(&[gtk::accessible::Property::Label("Default")]);
    let set = gtk::Button::builder()
        .label("Set as Default")
        .valign(gtk::Align::Center)
        .build();
    set.connect_clicked(glib::clone!(
        #[strong]
        manager,
        #[weak]
        device,
        move |_| manager.set_default(&device)
    ));
    device
        .bind_property("is-default", &check, "visible")
        .sync_create()
        .build();
    device
        .bind_property("is-default", &set, "visible")
        .invert_boolean()
        .sync_create()
        .build();
    default.add_suffix(&check);
    default.add_suffix(&set);
    group.add(&default);

    let channels = match device.channels() {
        0 => String::new(),
        1 => "Mono".to_owned(),
        2 => "Stereo".to_owned(),
        n => format!("{n} channels"),
    };
    for (label, value) in [
        ("Connection", device.connection()),
        ("Codec", device.codec()),
        ("Battery", device.battery()),
        ("Channels", channels),
        ("Device Name", device.name()),
    ] {
        if !value.is_empty() {
            group.add(
                &adw::ActionRow::builder()
                    .title(label)
                    .subtitle(&value)
                    .subtitle_selectable(true)
                    .use_markup(false)
                    .css_classes(["property"])
                    .build(),
            );
        }
    }
    group
}

/// The output's equalizer groups, added to `page`.
fn equalizer(
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    manager: &AudioManager,
    device: &AudioDevice,
) {
    let editor = Rc::new(Editor {
        manager: manager.clone(),
        device: device.clone(),
        dialog: dialog.downgrade(),
        settings: RefCell::new(manager.eq(device)),
        save_queued: Cell::new(false),
        syncing: Cell::new(false),
        preset: adw::ComboRow::builder().title("Preset").build(),
        preamp: band_scale(),
        bands: std::array::from_fn(|_| band_scale()),
        graphic: adw::PreferencesGroup::builder().title("Bands").build(),
        imported: adw::ActionRow::builder().use_markup(false).build(),
    });

    let switch = adw::SwitchRow::builder()
        .title("Equalizer")
        .subtitle("Shapes the sound of this output, even when Openhub is closed")
        .active(editor.settings.borrow().enabled)
        .build();
    switch.connect_active_notify(glib::clone!(
        #[weak]
        editor,
        move |switch| editor.change(|settings| settings.enabled = switch.is_active())
    ));
    let top = adw::PreferencesGroup::new();
    top.add(&switch);
    page.add(&top);

    let mut names: Vec<&str> = eq::PRESETS.iter().map(|(name, _)| *name).collect();
    names.push("Custom");
    editor.preset.set_model(Some(&gtk::StringList::new(&names)));
    editor.preset.connect_selected_notify(glib::clone!(
        #[weak]
        editor,
        move |row| {
            if editor.syncing.get() {
                return;
            }
            if let Some((_, gains)) = eq::PRESETS.get(row.selected() as usize) {
                editor.change(|settings| {
                    settings.filters = eq::graphic(gains);
                    settings.preamp = 0.0;
                });
                editor.sync();
            }
        }
    ));
    editor.graphic.add(&editor.preset);
    editor.graphic.add(&editor.sliders());
    page.add(&editor.graphic);

    let autoeq = adw::PreferencesGroup::builder()
        .title("AutoEQ")
        .description(
            "Correct a headphone or IEM with its ParametricEQ.txt file from the AutoEQ project",
        )
        .build();
    let import = gtk::Button::builder()
        .label("Import…")
        .valign(gtk::Align::Center)
        .build();
    import.connect_clicked(glib::clone!(
        #[weak]
        editor,
        move |_| editor.import()
    ));
    let import_row = adw::ActionRow::builder()
        .title("Import ParametricEQ.txt")
        .build();
    import_row.add_suffix(&import);
    autoeq.add(&import_row);
    let remove = gtk::Button::builder()
        .icon_name("user-trash-symbolic")
        .tooltip_text("Back to the graphic equalizer")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    remove.connect_clicked(glib::clone!(
        #[weak]
        editor,
        move |_| {
            editor.change(|settings| {
                *settings = Settings {
                    enabled: settings.enabled,
                    ..Settings::default()
                }
            });
            editor.sync();
        }
    ));
    editor.imported.add_suffix(&remove);
    autoeq.add(&editor.imported);
    page.add(&autoeq);

    // Bands follow the switch, so it is clear they do nothing while off.
    for widget in [
        editor.graphic.upcast_ref::<gtk::Widget>(),
        autoeq.upcast_ref(),
    ] {
        switch
            .bind_property("active", widget, "sensitive")
            .sync_create()
            .build();
    }
    editor.sync();
    // The dialog owns the editor; its widgets only point back weakly.
    dialog.connect_closed(move |_| {
        let _ = &editor;
    });
}

struct Editor {
    manager: AudioManager,
    device: AudioDevice,
    dialog: glib::WeakRef<adw::PreferencesDialog>,
    settings: RefCell<Settings>,
    save_queued: Cell<bool>,
    /// Set while widgets are updated from the settings, so their change
    /// handlers don't write back.
    syncing: Cell<bool>,
    preset: adw::ComboRow,
    preamp: gtk::Scale,
    bands: [gtk::Scale; 10],
    graphic: adw::PreferencesGroup,
    imported: adw::ActionRow,
}

impl Editor {
    /// The preamp and the 10 band sliders, side by side with their labels.
    fn sliders(self: &Rc<Self>) -> gtk::Widget {
        let row = gtk::Box::builder()
            .spacing(2)
            .homogeneous(true)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(6)
            .margin_end(6)
            .build();
        let labels =
            std::iter::once("Pre".to_owned()).chain(eq::GRAPHIC_FREQS.iter().map(|&freq| {
                if freq >= 1000.0 {
                    format!("{}k", freq / 1000.0)
                } else {
                    format!("{freq}")
                }
            }));
        let scales = std::iter::once(&self.preamp).chain(&self.bands);
        for (index, (scale, label)) in scales.zip(labels).enumerate() {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
            column.append(scale);
            column.append(
                &gtk::Label::builder()
                    .label(&label)
                    .css_classes(["caption"])
                    .build(),
            );
            scale.update_property(&[gtk::accessible::Property::Label(&if index == 0 {
                "Preamp".to_owned()
            } else {
                format!("{label}Hz")
            })]);
            scale.connect_value_changed(glib::clone!(
                #[weak(rename_to = editor)]
                self,
                move |scale| {
                    if editor.syncing.get() {
                        return;
                    }
                    let value = scale.value();
                    editor.change(|settings| match index {
                        0 => settings.preamp = value,
                        band => settings.filters[band - 1].gain = value,
                    });
                    editor.sync_preset();
                }
            ));
            row.append(&column);
        }
        let card = gtk::Box::builder().css_classes(["card"]).build();
        card.append(&row);
        row.set_hexpand(true);
        card.upcast()
    }

    /// Updates every widget from the settings.
    fn sync(&self) {
        self.syncing.set(true);
        let settings = self.settings.borrow();
        let gains = settings.graphic_gains();
        // After an import the bars show its curve at their frequencies,
        // read-only; Remove goes back to the editable graphic EQ.
        let editable = gains.is_some();
        self.preset.set_visible(editable);
        self.imported.set_visible(!editable);
        self.preamp.set_value(settings.preamp);
        self.preamp.set_sensitive(editable);
        let gains = gains
            .unwrap_or_else(|| eq::GRAPHIC_FREQS.map(|freq| eq::response(&settings.filters, freq)));
        for (scale, gain) in self.bands.iter().zip(gains) {
            scale.set_value(gain);
            scale.set_sensitive(editable);
        }
        if !editable {
            self.imported.set_title(&settings.imported);
            self.imported.set_subtitle(&format!(
                "{} filters, preamp {:.1} dB",
                settings.filters.len(),
                settings.preamp
            ));
        }
        drop(settings);
        self.syncing.set(false);
        self.sync_preset();
    }

    /// Selects the preset the bands match, or Custom.
    fn sync_preset(&self) {
        let Some(gains) = self.settings.borrow().graphic_gains() else {
            return;
        };
        let preamp = self.settings.borrow().preamp;
        let index = eq::PRESETS
            .iter()
            .position(|(_, preset)| *preset == gains && preamp == 0.0)
            .unwrap_or(eq::PRESETS.len());
        self.syncing.set(true);
        self.preset.set_selected(index as u32);
        self.syncing.set(false);
    }

    /// Edits the settings, then applies and saves them once per burst, so a
    /// dragged slider doesn't rewrite the files on every step.
    fn change(self: &Rc<Self>, edit: impl FnOnce(&mut Settings)) {
        edit(&mut self.settings.borrow_mut());
        if self.save_queued.replace(true) {
            return;
        }
        // Strong, so the last change is saved even if the dialog just closed.
        let editor = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
            editor.save_queued.set(false);
            let settings = editor.settings.borrow().clone();
            if let Err(error) = editor.manager.set_eq(&editor.device, settings) {
                editor.toast(&error);
            }
        });
    }

    fn import(self: &Rc<Self>) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Text files"));
        filter.add_mime_type("text/plain");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let chooser = gtk::FileDialog::builder()
            .title("Import ParametricEQ.txt")
            .filters(&filters)
            .build();
        let parent = self.dialog.upgrade().and_then(|dialog| dialog.root());
        let parent = parent.and_downcast_ref::<gtk::Window>();
        let editor = self.clone();
        chooser.open(parent, gio::Cancellable::NONE, move |file| {
            let Ok(file) = file else {
                return; // Cancelled.
            };
            let editor = editor.clone();
            let name = file
                .basename()
                .map(|name| name.display().to_string())
                .unwrap_or_default();
            file.load_contents_async(gio::Cancellable::NONE, move |result| {
                let parsed = result
                    .map_err(|error| error.to_string())
                    .and_then(|(bytes, _)| eq::parse_autoeq(&String::from_utf8_lossy(&bytes)));
                match parsed {
                    Ok((preamp, filters)) => {
                        editor.change(|settings| {
                            settings.preamp = preamp;
                            settings.filters = filters;
                            settings.imported = name;
                        });
                        editor.sync();
                    }
                    Err(error) => editor.toast(&error),
                }
            });
        });
    }

    fn toast(&self, title: &str) {
        if let Some(dialog) = self.dialog.upgrade() {
            dialog.add_toast(adw::Toast::builder().title(title).use_markup(false).build());
        }
    }
}

/// A vertical slider from −12 to +12 dB, with 0 dB marked.
fn band_scale() -> gtk::Scale {
    let scale =
        gtk::Scale::with_range(gtk::Orientation::Vertical, -eq::MAX_GAIN, eq::MAX_GAIN, 0.5);
    scale.set_inverted(true);
    scale.set_height_request(160);
    scale.set_halign(gtk::Align::Center);
    scale.add_mark(0.0, gtk::PositionType::Right, None);
    scale
}
