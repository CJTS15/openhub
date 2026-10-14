//! blueman-applet pops a notification for every connection change, and
//! rapid reconnects (such as after turning the adapter off and on) can make
//! GNOME Shell loop on one and freeze (#4). Turn that one blueman plugin off,
//! through the applet's own D-Bus API, which also saves the choice.

use zbus::proxy;

const APPLET: &str = "org.blueman.Applet";
const PLUGIN: &str = "ConnectionNotifier";

#[proxy(
    interface = "org.blueman.Applet",
    default_service = "org.blueman.Applet",
    default_path = "/org/blueman/Applet"
)]
trait Applet {
    fn query_plugins(&self) -> zbus::Result<Vec<String>>;

    fn set_plugin_config(&self, plugin: &str, enabled: bool) -> zbus::Result<()>;
}

/// Turns off blueman's connection notifications if blueman-applet is
/// running with them on. Returns whether it changed anything.
pub async fn disable_connection_notifier() -> zbus::Result<bool> {
    let conn = zbus::Connection::session().await?;
    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    // The applet is D-Bus activatable: calling it while it's not running would start blueman.
    if !dbus.name_has_owner(APPLET.try_into()?).await? {
        return Ok(false);
    }
    let applet = AppletProxy::new(&conn).await?;
    if !applet.query_plugins().await?.iter().any(|p| p == PLUGIN) {
        return Ok(false);
    }
    applet.set_plugin_config(PLUGIN, false).await?;
    Ok(true)
}
