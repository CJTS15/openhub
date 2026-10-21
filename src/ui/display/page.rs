use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use super::details_dialog;
use crate::display::{self, Monitor, State, edid};

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct DisplayPage {
        pub page: adw::PreferencesPage,
        /// The groups currently shown, replaced on each change.
        pub groups: RefCell<Vec<adw::PreferencesGroup>>,
        /// The Mutter serial shown, so a repeated signal doesn't rebuild.
        pub serial: Cell<Option<u32>>,
        pub subscription: RefCell<Option<gio::SignalSubscription>>,
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
            obj.set_child(Some(&self.page));
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
    /// The Displays page: every monitor, as Mutter (or sysfs) reports it.
    pub struct DisplayPage(ObjectSubclass<imp::DisplayPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl DisplayPage {
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
        self.reload(&bus).await;
    }

    async fn reload(&self, bus: &gio::DBusConnection) {
        match display::current_state(bus).await {
            Ok(state) => {
                if self.imp().serial.replace(Some(state.serial)) != Some(state.serial) {
                    self.show_state(&state);
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

    fn show_state(&self, state: &State) {
        let group = adw::PreferencesGroup::builder().title("Displays").build();
        // Built-in first, then the rest in Mutter's order.
        let mut monitors: Vec<&Monitor> = state.monitors.iter().collect();
        monitors.sort_by_key(|monitor| !monitor.builtin);
        for monitor in monitors {
            let edid = display::edid_for(&monitor.connector);
            let row = adw::ActionRow::builder()
                .title(title(monitor, edid.as_ref()))
                .subtitle(subtitle(state, monitor))
                .activatable(true)
                .use_markup(false)
                .build();
            row.add_prefix(&gtk::Image::from_icon_name(if monitor.builtin {
                "computer-symbolic"
            } else {
                "video-display-symbolic"
            }));
            let (monitor, logical) = (
                monitor.clone(),
                state.logical_for(&monitor.connector).cloned(),
            );
            row.connect_activated(move |row| {
                let edid = display::edid_for(&monitor.connector);
                details_dialog::new(&monitor, logical.as_ref(), edid.as_ref()).present(Some(row));
            });
            group.add(&row);
        }
        self.replace_groups(vec![group]);
    }

    /// Without Mutter: the monitors sysfs lists, read-only.
    fn show_fallback(&self) {
        self.imp().serial.set(None);
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
