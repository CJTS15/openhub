use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::model::PowerDevice;
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
        pub status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub level_bar: TemplateChild<gtk::LevelBar>,
        #[template_child]
        pub computer_list: TemplateChild<gtk::ListBox>,
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

            manager
                .bind_property("status", &*self.status_label, "label")
                .sync_create()
                .build();
            manager
                .bind_property("level", &*self.level_bar, "value")
                .sync_create()
                .build();
            manager
                .bind_property("has-battery", &*self.status_group, "visible")
                .sync_create()
                .build();
            self.computer_list
                .bind_model(Some(&manager.computer()), |item| {
                    row(item.downcast_ref().unwrap())
                });
            self.device_list
                .bind_model(Some(&manager.peripherals()), |item| {
                    row(item.downcast_ref().unwrap())
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

fn row(device: &PowerDevice) -> gtk::Widget {
    let row = adw::ActionRow::builder().use_markup(false).build();
    let icon = gtk::Image::new();
    for (source, target, widget) in [
        ("title", "title", row.upcast_ref::<glib::Object>()),
        ("subtitle", "subtitle", row.upcast_ref()),
        ("icon-name", "icon-name", icon.upcast_ref()),
    ] {
        device
            .bind_property(source, widget, target)
            .sync_create()
            .build();
    }
    row.add_prefix(&icon);
    row.upcast()
}
