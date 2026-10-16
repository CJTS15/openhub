use adw::prelude::*;
use gtk::{gio, glib};

use super::mouse_dialog::{self, message};
use crate::hidpp::{self, Node, Settings, State, Wheel};
use crate::model::InputDevice;

/// A Logitech mouse's HID++ settings: DPI, scroll wheel and direction.
/// Changes wait for Apply, then go to the mouse in one write and are saved,
/// so they come back after the mouse is switched off.
pub fn new(device: &InputDevice, node: Node) -> adw::PreferencesDialog {
    let (dialog, page, loading) = mouse_dialog::loading(device);

    glib::spawn_future_local(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        page,
        async move {
            let path = node.path.clone();
            let state = gio::spawn_blocking(move || hidpp::read(&path)).await;
            let owner = other_owner().await;
            page.remove(&loading);
            match state {
                Ok(Ok(state)) => fill(&dialog, &page, node, state, owner),
                Ok(Err(error)) => page.add(&message(&error.to_string())),
                Err(_) => page.add(&message("Reading the mouse failed")),
            }
        }
    ));
    dialog
}

fn fill(
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    node: Node,
    state: State,
    owner: Option<&'static str>,
) {
    let group = adw::PreferencesGroup::new();
    if let Some(owner) = owner {
        group.set_description(Some(&format!(
            "{owner} is managing this mouse. Close {owner} to change its settings here."
        )));
    }
    let settings = state.settings;

    let dpi = state
        .dpi_range
        .zip(settings.dpi)
        .map(|((low, high, step), dpi)| {
            let row = adw::SpinRow::with_range(low.into(), high.into(), step.into());
            row.set_title("Pointer Speed");
            if let Some(default) = state.dpi_default {
                row.set_subtitle(&format!("DPI · default {default}"));
            }
            row.set_value(dpi.into());
            group.add(&row);
            row
        });

    let wheel = settings.wheel.map(|wheel| {
        let mode = adw::ComboRow::builder()
            .title("Scroll Wheel")
            .model(&gtk::StringList::new(&[
                "SmartShift",
                "Ratchet",
                "Free Spin",
            ]))
            .build();
        mode.set_selected(match wheel {
            Wheel::SmartShift(_) => 0,
            Wheel::Ratchet => 1,
            Wheel::FreeSpin => 2,
        });
        let threshold = match wheel {
            Wheel::SmartShift(threshold) => threshold,
            _ => 10,
        };
        // Solaar's range; a mouse may report more, so it stays reachable.
        let sensitivity = adw::SpinRow::with_range(1.0, f64::from(threshold.max(50)), 1.0);
        sensitivity.set_title("SmartShift Threshold");
        sensitivity.set_subtitle("How fast a flick lets the wheel spin free");
        sensitivity.set_value(threshold.into());
        mode.bind_property("selected", &sensitivity, "visible")
            .transform_to(|_, selected: u32| Some(selected == 0))
            .sync_create()
            .build();
        group.add(&mode);
        group.add(&sensitivity);
        (mode, sensitivity)
    });

    let inverted = settings.inverted.map(|inverted| {
        let row = adw::SwitchRow::builder()
            .title("Reverse Scroll Direction")
            .active(inverted)
            .build();
        group.add(&row);
        row
    });
    page.add(&group);

    let apply = gtk::Button::builder()
        .label("Apply")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .sensitive(owner.is_none())
        .build();
    group.set_sensitive(owner.is_none());
    apply.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |button| {
            let wanted = Settings {
                dpi: dpi.as_ref().map(|row| row.value() as u16),
                wheel: wheel
                    .as_ref()
                    .map(|(mode, sensitivity)| match mode.selected() {
                        0 => Wheel::SmartShift(sensitivity.value() as u8),
                        1 => Wheel::Ratchet,
                        _ => Wheel::FreeSpin,
                    }),
                inverted: inverted.as_ref().map(adw::SwitchRow::is_active),
            };
            button.set_sensitive(false);
            let node = node.clone();
            glib::spawn_future_local(glib::clone!(
                #[weak]
                dialog,
                #[weak]
                button,
                async move {
                    let path = node.path.clone();
                    let written = gio::spawn_blocking(move || hidpp::write(&path, &wanted)).await;
                    button.set_sensitive(true);
                    let result = match written {
                        Ok(Ok(())) => hidpp::save(&node.uniq, wanted),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => Err("Writing to the mouse failed".to_owned()),
                    };
                    let title = result.err().unwrap_or_else(|| "Applied".to_owned());
                    dialog.add_toast(
                        adw::Toast::builder()
                            .title(&title)
                            .use_markup(false)
                            .build(),
                    );
                }
            ));
        }
    ));
    let footer = adw::PreferencesGroup::new();
    footer.add(&apply);
    page.add(&footer);
}

/// An app that already talks HID++ to Logitech mice; two at once confuse
/// the mouse. Solaar is a GTK app, so its name shows on the session bus.
// ponytail: OpenLogi's agent has no known bus name, so it isn't detected.
async fn other_owner() -> Option<&'static str> {
    let connection = zbus::Connection::session().await.ok()?;
    let dbus = zbus::fdo::DBusProxy::new(&connection).await.ok()?;
    let name = zbus::names::BusName::try_from("io.github.pwr_solaar.solaar").ok()?;
    dbus.name_has_owner(name).await.ok()?.then_some("Solaar")
}
