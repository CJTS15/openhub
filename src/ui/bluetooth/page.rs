use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use super::device_row;
use crate::bluez::{BluetoothManager, State};
use crate::model::{Adapter, Device};

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/cjts15/Openhub/ui/bluetooth_page.ui")]
    pub struct BluetoothPage {
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub adapter_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub device_list: TemplateChild<gtk::ListBox>,
        pub manager: BluetoothManager,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BluetoothPage {
        const NAME: &'static str = "OpenhubBluetoothPage";
        type Type = super::BluetoothPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BluetoothPage {
        fn constructed(&self) {
            self.parent_constructed();

            self.manager
                .bind_property("state", &*self.stack, "visible-child-name")
                .transform_to(|_, state: State| {
                    Some(match state {
                        State::Loading => "loading",
                        State::Unavailable => "unavailable",
                        State::NoAdapter => "no-adapter",
                        State::Ready => "ready",
                    })
                })
                .sync_create()
                .build();

            self.adapter_list
                .bind_model(Some(&self.manager.adapters()), |item| {
                    adapter_row(item.downcast_ref().unwrap()).upcast()
                });
            self.device_list
                .bind_model(Some(&self.manager.devices()), |item| {
                    let device: &Device = item.downcast_ref().unwrap();
                    device_row::new(device).upcast()
                });
        }
    }

    impl WidgetImpl for BluetoothPage {}
    impl BinImpl for BluetoothPage {}
}

glib::wrapper! {
    pub struct BluetoothPage(ObjectSubclass<imp::BluetoothPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

fn adapter_row(adapter: &Adapter) -> adw::ActionRow {
    let row = adw::ActionRow::builder().use_markup(false).build();
    row.add_prefix(&gtk::Image::from_icon_name("bluetooth-symbolic"));
    adapter
        .bind_property("name", &row, "title")
        .sync_create()
        .build();
    adapter
        .bind_property("powered", &row, "subtitle")
        .transform_to(|_, powered: bool| Some(if powered { "On" } else { "Off" }))
        .sync_create()
        .build();
    row
}
