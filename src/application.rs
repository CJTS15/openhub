use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config;
use crate::model::PowerDevice;
use crate::upower::PowerManager;
use crate::window::OpenhubWindow;

/// What the GNOME Shell extension reads while Openhub runs: the batteries
/// UPower doesn't list, as (name, UPower device kind, percentage).
const DEVICES_XML: &str = r#"
<node>
  <interface name="io.github.cjts15.Openhub.Devices">
    <method name="GetBatteries">
      <arg type="a(sud)" direction="out"/>
    </method>
  </interface>
</node>"#;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct OpenhubApplication {
        pub devices: std::cell::RefCell<Option<gio::RegistrationId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for OpenhubApplication {
        const NAME: &'static str = "OpenhubApplication";
        type Type = super::OpenhubApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for OpenhubApplication {}

    impl ApplicationImpl for OpenhubApplication {
        fn activate(&self) {
            let app = self.obj();
            // Single-instance: reuse the existing window if there is one.
            let window = app
                .active_window()
                .unwrap_or_else(|| OpenhubWindow::new(&*app).upcast());
            window.present();
        }

        fn dbus_register(
            &self,
            connection: &gio::DBusConnection,
            object_path: &str,
        ) -> Result<(), glib::Error> {
            self.parent_dbus_register(connection, object_path)?;
            let interface = gio::DBusNodeInfo::for_xml(DEVICES_XML)?
                .lookup_interface("io.github.cjts15.Openhub.Devices")
                .expect("the interface above");
            let id = connection
                .register_object(object_path, &interface)
                .method_call(|_, _, _, _, method, _, invocation| match method {
                    "GetBatteries" => {
                        let batteries: Vec<(String, u32, f64)> = PowerManager::shared()
                            .extra_batteries()
                            .iter()
                            .map(|d: &PowerDevice| (d.title(), d.kind(), d.level()))
                            .collect();
                        invocation.return_value(Some(&(batteries,).to_variant()));
                    }
                    _ => invocation.return_error(gio::IOErrorEnum::NotSupported, method),
                })
                .build()?;
            self.devices.replace(Some(id));
            Ok(())
        }

        fn dbus_unregister(&self, connection: &gio::DBusConnection, object_path: &str) {
            if let Some(id) = self.devices.take() {
                let _ = connection.unregister_object(id);
            }
            self.parent_dbus_unregister(connection, object_path);
        }

        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            app.setup_actions();
            app.setup_accels();
        }
    }

    impl GtkApplicationImpl for OpenhubApplication {}
    impl AdwApplicationImpl for OpenhubApplication {}
}

glib::wrapper! {
    pub struct OpenhubApplication(ObjectSubclass<imp::OpenhubApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl OpenhubApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", config::APP_ID)
            .property("resource-base-path", config::RESOURCE_BASE_PATH)
            .build()
    }

    fn setup_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();
        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.show_about())
            .build();
        // `app.show-page('bluetooth')`: opens Openhub on that page. The GNOME
        // Shell extension calls it over D-Bus (org.gtk.Actions).
        let show_page = gio::ActionEntry::builder("show-page")
            .parameter_type(Some(glib::VariantTy::STRING))
            .activate(|app: &Self, _, page| {
                app.activate();
                let page = page.and_then(|p| p.get::<String>()).unwrap_or_default();
                if let Some(window) = app.active_window().and_downcast::<OpenhubWindow>() {
                    window.show_category(&page);
                }
            })
            .build();
        self.add_action_entries([quit, about, show_page]);
    }

    fn setup_accels(&self) {
        self.set_accels_for_action("app.quit", &["<Control>q"]);
        self.set_accels_for_action("window.close", &["<Control>w"]);
    }

    fn show_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Openhub")
            .application_icon(config::APP_ID)
            .developer_name("CJ")
            .version(config::VERSION)
            .website("https://github.com/CJTS15/openhub")
            .issue_url("https://github.com/CJTS15/openhub/issues")
            .license_type(gtk::License::Gpl30)
            .build();
        dialog.present(self.active_window().as_ref());
    }
}

impl Default for OpenhubApplication {
    fn default() -> Self {
        Self::new()
    }
}
