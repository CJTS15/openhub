use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::PowerDevice;
use crate::upower::convert::percent;
use crate::upower::{PowerManager, State};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/power_page.ui")]
    pub struct PowerPage {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
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
    let level = gtk::Label::builder().css_classes(["dim-label"]).build();
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
        .bind_property("level", &level, "label")
        .transform_to(|_, level: f64| Some(percent(level)))
        .sync_create()
        .build();
    device
        .bind_property("level", &bar, "value")
        .transform_to(|_, level: f64| Some(level.max(0.0) / 100.0))
        .sync_create()
        .build();
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
    let row = gtk::ListBoxRow::builder()
        .activatable(false)
        .child(&content)
        .build();
    row.upcast()
}
