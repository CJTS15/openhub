use adw::prelude::*;
use gtk::glib;

use crate::ratbag::{self, Action, Changes, Mouse, SPECIALS};

/// A ratbagd mouse's DPI steps, report rate and buttons in the active
/// profile. Changes wait for Apply, then ratbagd writes them to the mouse.
pub fn fill(
    dialog: &adw::PreferencesDialog,
    page: &adw::PreferencesPage,
    conn: zbus::Connection,
    mouse: Mouse,
) {
    let speed = adw::PreferencesGroup::builder()
        .title("Pointer Speed")
        .build();
    let steps: Vec<adw::SpinRow> = mouse
        .resolutions
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let low = step.allowed.first().copied().unwrap_or(step.dpi);
            let high = step.allowed.last().copied().unwrap_or(step.dpi);
            let increment = match step.allowed.as_slice() {
                [a, b, ..] => b - a,
                _ => 50,
            };
            let row = adw::SpinRow::with_range(low.into(), high.into(), increment.into());
            row.set_title(&if mouse.resolutions.len() == 1 {
                "DPI".to_owned()
            } else {
                format!("DPI Step {}", index + 1)
            });
            let marks: Vec<&str> = [(step.active, "In use"), (step.default, "Default")]
                .into_iter()
                .filter_map(|(on, label)| on.then_some(label))
                .collect();
            row.set_subtitle(&marks.join(" · "));
            row.set_value(step.dpi.into());
            speed.add(&row);
            row
        })
        .collect();
    let rate = mouse.report_rate.as_ref().map(|(current, rates)| {
        let labels: Vec<String> = rates.iter().map(|rate| format!("{rate} Hz")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let row = adw::ComboRow::builder()
            .title("Report Rate")
            .subtitle("How often the mouse reports its movement")
            .model(&gtk::StringList::new(&labels))
            .build();
        row.set_selected(rates.iter().position(|r| r == current).unwrap_or(0) as u32);
        speed.add(&row);
        row
    });
    page.add(&speed);

    let buttons_group = adw::PreferencesGroup::builder().title("Buttons").build();
    let count = mouse.buttons.len() as u32;
    let buttons: Vec<(adw::ComboRow, Vec<Action>)> = mouse
        .buttons
        .iter()
        .map(|button| {
            let mut choices: Vec<Action> = Vec::new();
            if button.types.contains(&ratbag::ACTION_BUTTON) {
                choices.extend((1..=count).map(Action::Button));
            }
            if button.types.contains(&ratbag::ACTION_SPECIAL) {
                choices.extend(SPECIALS.iter().map(|(offset, _)| Action::special(*offset)));
            }
            if button.types.contains(&ratbag::ACTION_NONE) {
                choices.push(Action::None);
            }
            if !choices.contains(&button.action) {
                choices.insert(0, button.action);
            }
            let labels: Vec<String> = choices.iter().map(|a| a.label()).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let row = adw::ComboRow::builder()
                .title(format!("Button {}", button.index + 1))
                .model(&gtk::StringList::new(&labels))
                .build();
            row.set_selected(
                choices
                    .iter()
                    .position(|a| *a == button.action)
                    .unwrap_or(0) as u32,
            );
            // Keys and macros need an editor Openhub doesn't have.
            if button.action == Action::Other {
                row.set_subtitle("A key or macro; change it in Piper");
                row.set_sensitive(false);
            }
            buttons_group.add(&row);
            (row, choices)
        })
        .collect();
    if !buttons.is_empty() {
        page.add(&buttons_group);
    }

    let apply = gtk::Button::builder()
        .label("Apply")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    apply.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |button| {
            // Only what changed is sent.
            let mut changes = Changes::default();
            if let (Some(row), Some((current, rates))) = (&rate, &mouse.report_rate)
                && let Some(wanted) = rates.get(row.selected() as usize)
                && wanted != current
            {
                changes.report_rate = Some(*wanted);
            }
            for (row, step) in steps.iter().zip(&mouse.resolutions) {
                let wanted = ratbag::nearest(&step.allowed, row.value() as u32);
                if wanted != step.dpi {
                    changes
                        .resolutions
                        .push((step.path.clone(), wanted, step.separate_axes));
                }
            }
            for ((row, choices), current) in buttons.iter().zip(&mouse.buttons) {
                if let Some(wanted) = choices.get(row.selected() as usize)
                    && *wanted != current.action
                {
                    changes.buttons.push((current.path.clone(), *wanted));
                }
            }
            button.set_sensitive(false);
            let (conn, mouse) = (conn.clone(), mouse.clone());
            glib::spawn_future_local(glib::clone!(
                #[weak]
                dialog,
                #[weak]
                button,
                async move {
                    let result = ratbag::apply(&conn, &mouse, &changes).await;
                    button.set_sensitive(true);
                    let title = match result {
                        Ok(()) => "Applied".to_owned(),
                        Err(error) => format!("ratbagd refused the change: {error}"),
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
