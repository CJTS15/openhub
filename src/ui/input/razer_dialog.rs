use adw::prelude::*;
use gtk::{gio, glib};

use super::mouse_dialog::{self, message};
use crate::model::InputDevice;
use crate::razer::{self, Node, State};

const RATES: [u16; 3] = [125, 500, 1000];

/// A Razer mouse's DPI and polling rate, which it stores itself. Changes
/// wait for Apply, then go to the mouse in one write.
pub fn new(device: &InputDevice, node: Node) -> adw::PreferencesDialog {
    let (dialog, page, loading) = mouse_dialog::loading(device);

    glib::spawn_future_local(glib::clone!(
        #[weak]
        dialog,
        #[weak]
        page,
        async move {
            let read_node = node.clone();
            let state = gio::spawn_blocking(move || razer::read(&read_node)).await;
            page.remove(&loading);
            match state {
                Ok(Ok(state)) => fill(&dialog, &page, node, state),
                Ok(Err(error)) => page.add(&message(&error.to_string())),
                Err(_) => page.add(&message("Reading the mouse failed")),
            }
        }
    ));
    dialog
}

fn fill(dialog: &adw::PreferencesDialog, page: &adw::PreferencesPage, node: Node, state: State) {
    let group = adw::PreferencesGroup::new();
    let dpi = adw::SpinRow::with_range(100.0, state.max_dpi.into(), 50.0);
    dpi.set_title("Pointer Speed");
    dpi.set_subtitle("DPI");
    dpi.set_value(state.dpi.into());
    group.add(&dpi);

    let rate = state.poll_rate.map(|current| {
        let labels: Vec<String> = RATES.iter().map(|rate| format!("{rate} Hz")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let row = adw::ComboRow::builder()
            .title("Polling Rate")
            .subtitle("How often the mouse reports its movement")
            .model(&gtk::StringList::new(&labels))
            .build();
        row.set_selected(RATES.iter().position(|&r| r == current).unwrap_or(2) as u32);
        group.add(&row);
        row
    });

    if let Some(battery) = state.battery {
        group.add(
            &adw::ActionRow::builder()
                .title("Battery")
                .subtitle(format!("{battery}%"))
                .css_classes(["property"])
                .build(),
        );
    }
    page.add(&group);

    let apply = gtk::Button::builder()
        .label("Apply")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    apply.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |button| {
            let wanted_dpi = dpi.value() as u16;
            let wanted_rate = rate.as_ref().map(|row| RATES[row.selected() as usize]);
            button.set_sensitive(false);
            let node = node.clone();
            glib::spawn_future_local(glib::clone!(
                #[weak]
                dialog,
                #[weak]
                button,
                async move {
                    let written =
                        gio::spawn_blocking(move || razer::write(&node, wanted_dpi, wanted_rate))
                            .await;
                    button.set_sensitive(true);
                    let title = match written {
                        Ok(Ok(())) => "Applied".to_owned(),
                        Ok(Err(error)) => error.to_string(),
                        Err(_) => "Writing to the mouse failed".to_owned(),
                    };
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
