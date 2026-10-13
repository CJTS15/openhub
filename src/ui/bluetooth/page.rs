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
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub adapter_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub paired_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub scan_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub scan_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub nearby_list: TemplateChild<gtk::ListBox>,
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
            let obj = self.obj();

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

            self.adapter_list.bind_model(
                Some(&self.manager.adapters()),
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_panic]
                    move |item| obj.adapter_row(item.downcast_ref().unwrap()).upcast()
                ),
            );
            self.paired_list.bind_model(
                Some(&self.manager.paired_devices()),
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_panic]
                    move |item| {
                        let device: &Device = item.downcast_ref().unwrap();
                        let row = device_row::new(device);
                        let spinner = gtk::Spinner::new();
                        device
                            .bind_property("busy", &spinner, "spinning")
                            .sync_create()
                            .build();
                        device
                            .bind_property("busy", &spinner, "visible")
                            .sync_create()
                            .build();
                        row.add_suffix(&spinner);
                        row.add_suffix(&obj.connect_button(device));
                        row.upcast()
                    }
                ),
            );
            self.nearby_list
                .bind_model(Some(&self.manager.nearby_devices()), |item| {
                    device_row::new(item.downcast_ref().unwrap()).upcast()
                });

            self.manager
                .bind_property("discovering", &*self.scan_button, "label")
                .transform_to(|_, discovering: bool| {
                    Some(if discovering { "Stop" } else { "Scan" })
                })
                .sync_create()
                .build();
            self.manager
                .bind_property("discovering", &*self.scan_spinner, "spinning")
                .sync_create()
                .build();
            self.manager
                .bind_property("discovering", &*self.scan_spinner, "visible")
                .sync_create()
                .build();
            self.scan_button.connect_clicked(glib::clone!(
                #[weak]
                obj,
                move |_| obj.toggle_discovery()
            ));
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

impl BluetoothPage {
    fn toast(&self, title: &str) {
        let toast = adw::Toast::builder().title(title).use_markup(false).build();
        self.imp().toast_overlay.add_toast(toast);
    }

    fn adapter_row(&self, adapter: &Adapter) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder().use_markup(false).build();
        row.add_prefix(&gtk::Image::from_icon_name("bluetooth-symbolic"));
        adapter
            .bind_property("name", &row, "title")
            .sync_create()
            .build();
        adapter
            .bind_property("powered", &row, "active")
            .sync_create()
            .build();
        row.connect_active_notify(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            adapter,
            move |row| {
                let powered = row.is_active();
                // Equal when the change came from BlueZ through the binding above.
                if powered == adapter.powered() {
                    return;
                }
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    page,
                    #[weak]
                    adapter,
                    #[weak]
                    row,
                    async move {
                        let result = page.imp().manager.set_powered(&adapter, powered).await;
                        if let Err(err) = result {
                            glib::g_warning!("openhub", "Can't set Powered={powered}: {err}");
                            page.toast(if powered {
                                "Couldn't turn Bluetooth on"
                            } else {
                                "Couldn't turn Bluetooth off"
                            });
                            row.set_active(adapter.powered());
                        }
                    }
                ));
            }
        ));
        row
    }

    fn connect_button(&self, device: &Device) -> gtk::Button {
        let button = gtk::Button::builder().valign(gtk::Align::Center).build();
        device
            .bind_property("connected", &button, "label")
            .transform_to(|_, connected: bool| {
                Some(if connected { "Disconnect" } else { "Connect" })
            })
            .sync_create()
            .build();
        // Busy lives on the device: a device that connects moves to the top,
        // which replaces this row and button.
        device
            .bind_property("busy", &button, "sensitive")
            .invert_boolean()
            .sync_create()
            .build();
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            move |_| {
                device.set_busy(true);
                let connected = device.connected();
                glib::spawn_future_local(glib::clone!(
                    #[weak]
                    page,
                    #[weak]
                    device,
                    async move {
                        let manager = &page.imp().manager;
                        let result = if connected {
                            manager.disconnect_device(&device).await
                        } else {
                            manager.connect_device(&device).await
                        };
                        if let Err(err) = result {
                            glib::g_warning!(
                                "openhub",
                                "Can't (dis)connect {}: {err}",
                                device.path()
                            );
                            page.toast(&if connected {
                                format!("Couldn't disconnect {}", device.name())
                            } else {
                                format!("Couldn't connect to {}", device.name())
                            });
                        }
                        device.set_busy(false);
                    }
                ));
            }
        ));
        button
    }

    fn toggle_discovery(&self) {
        let discovering = !self.imp().manager.discovering();
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            async move {
                if let Err(err) = page.imp().manager.set_discovering(discovering).await {
                    glib::g_warning!("openhub", "Can't set discovery to {discovering}: {err}");
                    page.toast(if discovering {
                        "Couldn't start scanning"
                    } else {
                        "Couldn't stop scanning"
                    });
                }
            }
        ));
    }
}
