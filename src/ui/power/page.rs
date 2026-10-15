use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::battery_dialog;
use crate::model::PowerDevice;
use crate::upower::convert::{percent, warning};
use crate::upower::{PowerManager, State};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/power_page.ui")]
    pub struct PowerPage {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub battery_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub charging_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub maximize_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub maximize_check: TemplateChild<gtk::CheckButton>,
        #[template_child]
        pub preserve_row: TemplateChild<adw::ActionRow>,
        #[template_child]
        pub preserve_check: TemplateChild<gtk::CheckButton>,
        /// The battery the charging rows follow, and its notify handler.
        pub watched_battery: RefCell<Option<(PowerDevice, glib::SignalHandlerId)>>,
        /// Set while the rows are updated from UPower, so that isn't taken
        /// as the user choosing.
        pub syncing: Cell<bool>,
        #[template_child]
        pub status_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub level_bar: TemplateChild<gtk::LevelBar>,
        #[template_child]
        pub status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub level_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub device_list: TemplateChild<gtk::ListBox>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PowerPage {
        const NAME: &'static str = "OpenhubPowerPage";
        type Type = super::PowerPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PowerPage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            let manager = PowerManager::shared();

            style_battery_bar(&self.level_bar);
            manager
                .bind_property("status", &*self.status_label, "label")
                .sync_create()
                .build();
            manager
                .bind_property("level", &*self.level_bar, "value")
                .transform_to(|_, level: f64| Some(level / 100.0))
                .sync_create()
                .build();
            manager
                .bind_property("level", &*self.level_label, "label")
                .transform_to(|_, level: f64| Some(percent(level)))
                .sync_create()
                .build();
            manager
                .bind_property("has-battery", &*self.status_group, "visible")
                .sync_create()
                .build();
            self.device_list
                .bind_model(Some(&manager.peripherals()), |item| {
                    device_row(item.downcast_ref().unwrap())
                });
            self.device_list.connect_row_activated(glib::clone!(
                #[weak]
                obj,
                move |_, row| {
                    let device = PowerManager::shared()
                        .peripherals()
                        .item(row.index() as u32);
                    if let Some(device) = device.and_downcast::<PowerDevice>() {
                        battery_dialog::new(&device).present(Some(&obj));
                    }
                }
            ));
            self.battery_list.connect_row_activated(glib::clone!(
                #[weak]
                obj,
                move |_, _| {
                    if let Some(battery) = PowerManager::shared().battery() {
                        battery_dialog::new(&battery).present(Some(&obj));
                    }
                }
            ));

            self.maximize_row
                .set_subtitle("Uses all battery capacity. Degrades batteries more quickly.");
            manager.connect_battery_notify(glib::clone!(
                #[weak]
                obj,
                move |_| obj.follow_battery()
            ));
            obj.follow_battery();
            // In a group, choosing Maximize Charge also unchecks this one.
            self.preserve_check.connect_toggled(glib::clone!(
                #[weak]
                obj,
                move |check| obj.choose_charge_limit(check.is_active())
            ));

            let update = glib::clone!(
                #[weak]
                obj,
                move || obj.update_stack()
            );
            manager.connect_state_notify(glib::clone!(
                #[strong]
                update,
                move |_| update()
            ));
            for store in [manager.computer(), manager.peripherals()] {
                store.connect_items_changed(glib::clone!(
                    #[strong]
                    update,
                    move |_, _, _, _| update()
                ));
            }
            update();
        }
    }

    impl WidgetImpl for PowerPage {}
    impl BinImpl for PowerPage {}
}

glib::wrapper! {
    pub struct PowerPage(ObjectSubclass<imp::PowerPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PowerPage {
    /// Points the Battery Charging rows at the current main battery.
    fn follow_battery(&self) {
        let imp = self.imp();
        if let Some((battery, handler)) = imp.watched_battery.take() {
            battery.disconnect(handler);
        }
        if let Some(battery) = PowerManager::shared().battery() {
            let handler = battery.connect_notify_local(
                None,
                glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_, _| page.sync_charging()
                ),
            );
            imp.watched_battery.replace(Some((battery, handler)));
        }
        self.sync_charging();
    }

    /// Shows the charge limit as UPower reports it; hidden when unsupported.
    fn sync_charging(&self) {
        let imp = self.imp();
        let battery = PowerManager::shared().battery();
        let supported = battery
            .as_ref()
            .is_some_and(PowerDevice::charge_limit_supported);
        imp.charging_group.set_visible(supported);
        let Some(battery) = battery.filter(|_| supported) else {
            return;
        };
        imp.preserve_row.set_subtitle(&format!(
            "Increases battery longevity by stopping charging at {}",
            percent(battery.charge_end_threshold().into())
        ));
        imp.syncing.set(true);
        let enabled = battery.charge_limit_enabled();
        imp.preserve_check.set_active(enabled);
        imp.maximize_check.set_active(!enabled);
        imp.syncing.set(false);
    }

    fn choose_charge_limit(&self, enabled: bool) {
        let manager = PowerManager::shared();
        let Some(battery) = manager.battery() else {
            return;
        };
        if self.imp().syncing.get() || battery.charge_limit_enabled() == enabled {
            return;
        }
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                if let Err(err) = manager.set_charge_limit(&battery, enabled).await {
                    glib::g_warning!("openhub", "Can't set the charge limit to {enabled}: {err}");
                    let toast = adw::Toast::new("Couldn't change battery charging");
                    page.imp().toast_overlay.add_toast(toast);
                    // Put the rows back to what UPower still reports.
                    page.sync_charging();
                }
            }
        ));
    }

    fn update_stack(&self) {
        let manager = PowerManager::shared();
        let empty = manager.computer().n_items() == 0 && manager.peripherals().n_items() == 0;
        self.imp()
            .stack
            .set_visible_child_name(match manager.state() {
                State::Loading => "loading",
                State::Unavailable => "unavailable",
                State::Ready if empty => "empty",
                State::Ready => "ready",
            });
    }
}

/// Fill runs 0 to 1, in the warning colour at 10% or less, as in GNOME Settings.
fn style_battery_bar(bar: &gtk::LevelBar) {
    bar.add_css_class("battery");
    for name in ["low", "high", "full"] {
        bar.remove_offset_value(Some(name));
    }
    bar.add_offset_value("low", 0.1);
    bar.add_offset_value("high", 1.0);
}

/// Name, percentage and a bar filling the rest of the row, as GNOME
/// Settings shows connected devices.
fn device_row(device: &PowerDevice) -> gtk::Widget {
    let name = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    let level = gtk::Label::new(None);
    let bar = gtk::LevelBar::builder()
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    style_battery_bar(&bar);
    device
        .bind_property("title", &name, "label")
        .sync_create()
        .build();
    device
        .bind_property("level-text", &level, "label")
        .sync_create()
        .build();
    device
        .bind_property("fraction", &bar, "value")
        .sync_create()
        .build();
    // Dim normally; the warning or error colour when UPower says it's low.
    let colour = glib::clone!(
        #[weak]
        level,
        move |device: &PowerDevice| {
            let warning_level = device.warning_level();
            let class = match warning_level {
                warning::CRITICAL | warning::ACTION => "error",
                warning::LOW => "warning",
                _ => "dim-label",
            };
            level.set_css_classes(&[class]);
        }
    );
    colour(device);
    device.connect_warning_level_notify(colour);
    let content = gtk::Box::builder()
        .spacing(12)
        .margin_start(12)
        .margin_end(12)
        .margin_top(15)
        .margin_bottom(15)
        .build();
    content.append(&name);
    content.append(&level);
    content.append(&bar);
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.upcast()
}
