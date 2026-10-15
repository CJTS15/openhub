use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config;
use crate::window::OpenhubWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct OpenhubApplication;

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
        self.add_action_entries([quit, about]);
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
