//! Keeping GObjects in step with D-Bus objects: each property stream from a
//! cached zbus proxy becomes a tick, and any tick reruns an update closure.

use futures_util::stream::{self, LocalBoxStream, Stream, StreamExt};
use gtk::glib;

pub fn ticks(stream: impl Stream + 'static) -> LocalBoxStream<'static, ()> {
    stream.map(|_| ()).boxed_local()
}

/// Runs `update` now and again whenever any of `changes` fires.
pub fn watch(
    changes: Vec<LocalBoxStream<'static, ()>>,
    update: impl Fn() + 'static,
) -> glib::JoinHandle<()> {
    update();
    glib::spawn_future_local(async move {
        let mut changes = stream::select_all(changes);
        while changes.next().await.is_some() {
            update();
        }
    })
}
