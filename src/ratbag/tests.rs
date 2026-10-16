//! Reads and writes a fake ratbagd: the same interfaces, served in-process
//! over a socket pair, so no bus daemon or real mouse is needed.

use std::sync::{Arc, Mutex};

use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::*;

/// What the client changed, as the fake daemon saw it.
#[derive(Default)]
struct Seen {
    report_rate: u32,
    dpi: Vec<OwnedValue>,
    mapping: Option<(u32, u32)>,
    committed: bool,
}

type State = Arc<Mutex<Seen>>;

fn path(text: &str) -> OwnedObjectPath {
    OwnedObjectPath::try_from(text).unwrap()
}

struct FakeManager;

#[zbus::interface(name = "org.freedesktop.ratbag1.Manager")]
impl FakeManager {
    #[zbus(property, name = "APIVersion")]
    fn api_version(&self) -> i32 {
        2
    }

    #[zbus(property)]
    fn devices(&self) -> Vec<OwnedObjectPath> {
        vec![path("/org/freedesktop/ratbag1/device/mouse")]
    }
}

struct FakeDevice(State);

#[zbus::interface(name = "org.freedesktop.ratbag1.Device")]
impl FakeDevice {
    #[zbus(property)]
    fn model(&self) -> String {
        "usb:1038:1836:0".into()
    }

    #[zbus(property)]
    fn name(&self) -> String {
        "SteelSeries Rival 3".into()
    }

    #[zbus(property)]
    fn profiles(&self) -> Vec<OwnedObjectPath> {
        vec![path("/org/freedesktop/ratbag1/profile/mouse/p0")]
    }

    fn commit(&self) {
        self.0.lock().unwrap().committed = true;
    }
}

struct FakeProfile(State);

#[zbus::interface(name = "org.freedesktop.ratbag1.Profile")]
impl FakeProfile {
    #[zbus(property)]
    fn is_active(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn resolutions(&self) -> Vec<OwnedObjectPath> {
        vec![
            path("/org/freedesktop/ratbag1/resolution/mouse/p0/r0"),
            path("/org/freedesktop/ratbag1/resolution/mouse/p0/r1"),
        ]
    }

    #[zbus(property)]
    fn buttons(&self) -> Vec<OwnedObjectPath> {
        vec![
            path("/org/freedesktop/ratbag1/button/mouse/p0/b0"),
            path("/org/freedesktop/ratbag1/button/mouse/p0/b5"),
        ]
    }

    #[zbus(property)]
    fn report_rate(&self) -> u32 {
        1000
    }

    #[zbus(property)]
    fn set_report_rate(&mut self, rate: u32) {
        self.0.lock().unwrap().report_rate = rate;
    }

    #[zbus(property)]
    fn report_rates(&self) -> Vec<u32> {
        vec![125, 250, 500, 1000]
    }
}

/// `axes`: this step exports (x, y) rather than one number.
struct FakeResolution {
    state: State,
    dpi: u32,
    axes: bool,
    active: bool,
}

#[zbus::interface(name = "org.freedesktop.ratbag1.Resolution")]
impl FakeResolution {
    #[zbus(property)]
    fn is_active(&self) -> bool {
        self.active
    }

    #[zbus(property)]
    fn is_default(&self) -> bool {
        self.active
    }

    #[zbus(property)]
    fn is_disabled(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn resolution(&self) -> OwnedValue {
        let value = if self.axes {
            Value::from((self.dpi, self.dpi))
        } else {
            Value::U32(self.dpi)
        };
        OwnedValue::try_from(value).unwrap()
    }

    #[zbus(property)]
    fn set_resolution(&mut self, value: OwnedValue) {
        self.state.lock().unwrap().dpi.push(value);
    }

    #[zbus(property)]
    fn resolutions(&self) -> Vec<u32> {
        (1..=72).map(|n| n * 100).collect()
    }
}

struct FakeButton {
    state: State,
    index: u32,
    mapping: (u32, u32),
}

#[zbus::interface(name = "org.freedesktop.ratbag1.Button")]
impl FakeButton {
    #[zbus(property)]
    fn index(&self) -> u32 {
        self.index
    }

    #[zbus(property)]
    fn mapping(&self) -> (u32, OwnedValue) {
        (self.mapping.0, OwnedValue::from(self.mapping.1))
    }

    #[zbus(property)]
    fn set_mapping(&mut self, mapping: (u32, OwnedValue)) {
        // zvariant gives the `v` field back as a nested value.
        let value = match &*mapping.1 {
            Value::Value(inner) => u32::try_from(&**inner),
            value => u32::try_from(value),
        }
        .unwrap_or(u32::MAX);
        self.state.lock().unwrap().mapping = Some((mapping.0, value));
    }

    #[zbus(property)]
    fn action_types(&self) -> Vec<u32> {
        vec![0, 1, 2, 3, 4]
    }
}

/// A client connected to a fake ratbagd with one mouse.
async fn fake_ratbagd(state: &State) -> (zbus::Connection, zbus::Connection) {
    let (server_end, client_end) = std::os::unix::net::UnixStream::pair().unwrap();
    let resolution = |dpi, axes, active| FakeResolution {
        state: state.clone(),
        dpi,
        axes,
        active,
    };
    let server = zbus::connection::Builder::unix_stream(server_end)
        .server(zbus::Guid::generate())
        .unwrap()
        .p2p()
        .serve_at("/org/freedesktop/ratbag1", FakeManager)
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/device/mouse",
            FakeDevice(state.clone()),
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/profile/mouse/p0",
            FakeProfile(state.clone()),
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/resolution/mouse/p0/r0",
            resolution(800, false, true),
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/resolution/mouse/p0/r1",
            resolution(1600, true, false),
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/button/mouse/p0/b0",
            FakeButton {
                state: state.clone(),
                index: 0,
                mapping: (ACTION_BUTTON, 1),
            },
        )
        .unwrap()
        .serve_at(
            "/org/freedesktop/ratbag1/button/mouse/p0/b5",
            FakeButton {
                state: state.clone(),
                index: 5,
                mapping: (ACTION_SPECIAL, SPECIAL + 7),
            },
        )
        .unwrap()
        .build();
    let client = zbus::connection::Builder::unix_stream(client_end)
        .p2p()
        .build();
    let (server, client) = futures_util::future::join(server, client).await;
    (server.unwrap(), client.unwrap())
}

#[test]
fn reads_and_writes_a_ratbagd_mouse() {
    zbus::block_on(async {
        let state = State::default();
        let (_server, client) = fake_ratbagd(&state).await;

        assert_eq!(find(&client, 0x046D, 0xC08B).await.unwrap(), None);
        let mouse = find(&client, 0x1038, 0x1836).await.unwrap().unwrap();
        assert_eq!(mouse.name, "SteelSeries Rival 3");
        assert_eq!(mouse.report_rate, Some((1000, vec![125, 250, 500, 1000])));
        let steps: Vec<_> = mouse
            .resolutions
            .iter()
            .map(|r| (r.dpi, r.separate_axes, r.active))
            .collect();
        assert_eq!(steps, [(800, false, true), (1600, true, false)]);
        assert_eq!(mouse.buttons[0].action, Action::Button(1));
        assert_eq!(mouse.buttons[1].action, Action::special(7));
        assert_eq!(mouse.buttons[1].action.label(), "DPI Cycle Up");

        let changes = Changes {
            report_rate: Some(500),
            resolutions: vec![
                (mouse.resolutions[0].path.clone(), 1200, false),
                (mouse.resolutions[1].path.clone(), 3200, true),
            ],
            buttons: vec![(mouse.buttons[1].path.clone(), Action::Button(3))],
        };
        apply(&client, &mouse, &changes).await.unwrap();
        let seen = state.lock().unwrap();
        assert_eq!(seen.report_rate, 500);
        // Each step keeps its form: one number, or (x, y).
        assert_eq!(u32::try_from(&seen.dpi[0]).ok(), Some(1200));
        assert_eq!(
            <(u32, u32)>::try_from(seen.dpi[1].try_clone().unwrap()).ok(),
            Some((3200, 3200))
        );
        assert_eq!(seen.mapping, Some((ACTION_BUTTON, 3)));
        assert!(seen.committed);
    });
}

#[test]
fn ids_and_nearest_values() {
    assert_eq!(usb_ids("usb:1038:1836:0"), Some((0x1038, 0x1836)));
    assert_eq!(usb_ids("bluetooth:046d:b034:0"), None);
    assert_eq!(usb_ids("unknown"), None);
    assert_eq!(nearest(&[400, 800, 1600], 1000), 800);
    assert_eq!(nearest(&[], 1000), 1000);
}
