use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use std::cell::RefCell;

use super::{device_dialog, device_row, pairing_dialog};
use crate::bluez::{BluetoothManager, PairingRequest, State};
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
        /// The open pairing dialog, closed when pairing ends or BlueZ cancels.
        pub pairing_dialog: RefCell<Option<adw::AlertDialog>>,
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
                        row.add_suffix(&busy_spinner(device));
                        row.add_suffix(&obj.connect_button(device));
                        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
                        row.set_activatable(true);
                        row.connect_activated(glib::clone!(
                            #[weak]
                            obj,
                            #[weak]
                            device,
                            move |_| {
                                device_dialog::new(&obj.imp().manager, &device).present(Some(&obj));
                            }
                        ));
                        row.upcast()
                    }
                ),
            );
            self.nearby_list.bind_model(
                Some(&self.manager.nearby_devices()),
                glib::clone!(
                    #[weak]
                    obj,
                    #[upgrade_or_panic]
                    move |item| {
                        let device: &Device = item.downcast_ref().unwrap();
                        let row = device_row::new(device);
                        row.add_suffix(&busy_spinner(device));
                        row.set_activatable(true);
                        row.connect_activated(glib::clone!(
                            #[weak]
                            obj,
                            #[weak]
                            device,
                            move |_| obj.pair(&device)
                        ));
                        row.upcast()
                    }
                ),
            );
            self.manager.connect_pairing_request(glib::clone!(
                #[weak]
                obj,
                move |request| obj.on_pairing_request(request)
            ));

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

fn busy_spinner(device: &Device) -> gtk::Spinner {
    let spinner = gtk::Spinner::new();
    device
        .bind_property("busy", &spinner, "spinning")
        .sync_create()
        .build();
    device
        .bind_property("busy", &spinner, "visible")
        .sync_create()
        .build();
    spinner
}

/// The user (or the device) said no; they already know, so no toast.
fn declined(err: &zbus::Error) -> bool {
    matches!(err, zbus::Error::MethodError(name, _, _)
        if name.ends_with(".AuthenticationCanceled") || name.ends_with(".AuthenticationRejected"))
}

impl BluetoothPage {
    fn pair(&self, device: &Device) {
        if device.busy() {
            return;
        }
        device.set_busy(true);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = page)]
            self,
            #[weak]
            device,
            async move {
                let manager = &page.imp().manager;
                // Scanning shares the radio with pairing and connecting, slowing
                // both; pause it and resume afterwards for the next device.
                let was_scanning = manager.discovering();
                if was_scanning {
                    if let Err(err) = manager.set_discovering(false).await {
                        glib::g_warning!("openhub", "Can't stop scanning to pair: {err}");
                    }
                }
                let result = manager.pair_device(&device).await;
                if let Some(dialog) = page.imp().pairing_dialog.take() {
                    dialog.force_close();
                }
                match result {
                    Ok(()) => {
                        if let Err(err) = manager.connect_device(&device).await {
                            glib::g_warning!("openhub", "Can't connect {}: {err}", device.path());
                            page.toast(&format!(
                                "Paired with {}, but couldn't connect",
                                device.name()
                            ));
                        }
                    }
                    Err(err) if declined(&err) => {}
                    Err(err) => {
                        glib::g_warning!("openhub", "Can't pair {}: {err}", device.path());
                        page.toast(&format!("Couldn't pair with {}", device.name()));
                    }
                }
                if was_scanning {
                    if let Err(err) = manager.set_discovering(true).await {
                        glib::g_warning!("openhub", "Can't resume scanning: {err}");
                    }
                }
                device.set_busy(false);
            }
        ));
    }

    fn on_pairing_request(&self, request: PairingRequest<Device>) {
        let current = self.imp().pairing_dialog.take();
        // DisplayPasskey repeats as the user types; update the open dialog.
        if let (PairingRequest::Display { device, code }, Some(dialog)) = (&request, &current) {
            dialog.set_body(&pairing_dialog::display_body(device, code));
            self.imp().pairing_dialog.replace(current);
            return;
        }
        if let Some(dialog) = current {
            dialog.force_close();
        }
        let display_device = match &request {
            PairingRequest::Display { device, .. } => Some(device.clone()),
            _ => None,
        };
        let Some(dialog) = pairing_dialog::new(request) else {
            return;
        };
        if let Some(device) = display_device {
            dialog.connect_response(
                Some("cancel"),
                glib::clone!(
                    #[weak(rename_to = page)]
                    self,
                    move |_, _| {
                        let device = device.clone();
                        glib::spawn_future_local(async move {
                            let _ = page.imp().manager.cancel_pairing(&device).await;
                        });
                    }
                ),
            );
        }
        dialog.connect_closed(glib::clone!(
            #[weak(rename_to = page)]
            self,
            move |dialog| {
                let mut current = page.imp().pairing_dialog.borrow_mut();
                if current.as_ref() == Some(dialog) {
                    current.take();
                }
            }
        ));
        dialog.present(Some(self));
        self.imp().pairing_dialog.replace(Some(dialog));
    }

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
