//! Logitech's HID++ 2.0 over `/dev/hidraw*`: DPI (feature 0x2201),
//! SmartShift (0x2110, or 0x2111 on newer mice) and scroll direction
//! (0x2121). Solaar's protocol notes are the reference. Calls block for up
//! to a few seconds, so run them off the main thread.

mod keeper;

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use keeper::Keeper;

/// A mouse paired directly (Bluetooth or cable) answers on this index.
// ponytail: mice behind a Bolt or Unifying receiver use indexes 1 to 6;
// they need receiver pairing lookup, added when there's one to test with.
const DIRECT: u8 = 0xFF;
/// Marks Openhub's requests, so replies aren't confused with the mouse's
/// own notifications or another app's traffic.
const SOFTWARE_ID: u8 = 0x0A;
const LONG_REPORT: u8 = 0x11;
const LOGITECH: u32 = 0x046D;

/// A Logitech HID++ device node, from sysfs.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub path: PathBuf,
    /// Bluetooth address (lowercase) or USB serial; matches `InputDevice::uniq`.
    pub uniq: String,
    pub name: String,
}

/// Logitech mice connected directly over Bluetooth or USB.
pub fn find() -> Vec<Node> {
    find_in(Path::new("/sys/class/hidraw"), Path::new("/dev"))
}

fn find_in(sys: &Path, dev: &Path) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(sys) else {
        return Vec::new();
    };
    let mut nodes: Vec<Node> = entries
        .flatten()
        .filter_map(|entry| {
            let uevent = std::fs::read_to_string(entry.path().join("device/uevent")).ok()?;
            let value = |key: &str| {
                uevent
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                    .unwrap_or_default()
                    .to_owned()
            };
            // HID_ID=0005:0000046D:0000B034 — bus 3 is USB, 5 is Bluetooth.
            let id = value("HID_ID");
            let mut parts = id.split(':');
            let bus = parts.next()?;
            let vendor = u32::from_str_radix(parts.next()?, 16).ok()?;
            let direct = matches!(bus, "0003" | "0005");
            (direct && vendor == LOGITECH).then(|| Node {
                path: dev.join(entry.file_name()),
                uniq: value("HID_UNIQ").to_lowercase(),
                name: value("HID_NAME"),
            })
        })
        .filter(|node| !node.uniq.is_empty())
        .collect();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    nodes
}

/// How the scroll wheel clicks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Wheel {
    /// Ratchets, and spins free when flicked faster than the threshold.
    SmartShift(u8),
    Ratchet,
    FreeSpin,
}

impl Wheel {
    /// From getRatchetControlMode's wheel mode and auto-disengage bytes.
    fn from_bytes(mode: u8, threshold: u8) -> Self {
        match (mode, threshold) {
            (1, _) => Wheel::FreeSpin,
            (_, 0xFF) => Wheel::Ratchet,
            (_, threshold) => Wheel::SmartShift(threshold),
        }
    }

    /// setRatchetControlMode's parameters; a zero leaves that part as is.
    fn to_bytes(self) -> [u8; 3] {
        match self {
            Wheel::SmartShift(threshold) => [2, threshold.clamp(1, 0xFE), 0],
            Wheel::Ratchet => [2, 0xFF, 0],
            Wheel::FreeSpin => [1, 0, 0],
        }
    }
}

/// What Openhub can change on a mouse. `None` where the mouse lacks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub dpi: Option<u16>,
    pub wheel: Option<Wheel>,
    pub inverted: Option<bool>,
}

/// A mouse's current settings and what it accepts.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub settings: Settings,
    /// Lowest, highest and step, such as 200, 8000 and 50.
    pub dpi_range: Option<(u16, u16, u16)>,
    pub dpi_default: Option<u16>,
}

struct Device {
    file: File,
}

impl Device {
    fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Ok(Self { file })
    }

    /// Sends a long report and waits for its reply's parameters.
    fn call(&mut self, feature: u8, function: u8, params: &[u8]) -> io::Result<[u8; 16]> {
        let mut report = [0u8; 20];
        report[..4].copy_from_slice(&[LONG_REPORT, DIRECT, feature, function << 4 | SOFTWARE_ID]);
        report[4..4 + params.len()].copy_from_slice(params);
        self.file.write_all(&report)?;
        loop {
            if !wait_readable(&self.file, Duration::from_secs(2))? {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "The mouse didn't answer. Move it to wake it up.",
                ));
            }
            let mut reply = [0u8; 64];
            let length = self.file.read(&mut reply)?;
            if length < 20 || reply[0] != LONG_REPORT || reply[1] != DIRECT {
                continue;
            }
            // An error reply: feature 0xFF, then the failed request's
            // feature and function, then the error code.
            if reply[2] == 0xFF && reply[3] == feature && reply[4] == function << 4 | SOFTWARE_ID {
                return Err(io::Error::other(format!("HID++ error {:#04x}", reply[5])));
            }
            if reply[2] == feature && reply[3] == function << 4 | SOFTWARE_ID {
                let mut out = [0u8; 16];
                out.copy_from_slice(&reply[4..20]);
                return Ok(out);
            }
        }
    }

    /// The feature's index on this mouse, from the root feature.
    fn feature(&mut self, id: u16) -> io::Result<Option<u8>> {
        let index = self.call(0x00, 0, &id.to_be_bytes())?[0];
        Ok((index != 0).then_some(index))
    }

    /// The SmartShift feature's index and its get/set functions; 0x2111
    /// moved them up by one.
    fn smartshift(&mut self) -> io::Result<Option<(u8, u8, u8)>> {
        if let Some(index) = self.feature(0x2110)? {
            return Ok(Some((index, 0, 1)));
        }
        Ok(self.feature(0x2111)?.map(|index| (index, 1, 2)))
    }
}

fn wait_readable(file: &File, timeout: Duration) -> io::Result<bool> {
    let fd = file.as_fd();
    let mut fds = [rustix::event::PollFd::new(
        &fd,
        rustix::event::PollFlags::IN,
    )];
    let timeout = rustix::event::Timespec::try_from(timeout).map_err(io::Error::other)?;
    Ok(rustix::event::poll(&mut fds, Some(&timeout))? > 0)
}

pub fn read(path: &Path) -> io::Result<State> {
    let mut device = Device::open(path)?;
    let mut state = State {
        settings: Settings {
            dpi: None,
            wheel: None,
            inverted: None,
        },
        dpi_range: None,
        dpi_default: None,
    };
    if let Some(dpi) = device.feature(0x2201)? {
        state.dpi_range = dpi_range(&device.call(dpi, 1, &[0])?);
        let reply = device.call(dpi, 2, &[0])?;
        state.settings.dpi = Some(u16::from_be_bytes([reply[1], reply[2]]));
        state.dpi_default = Some(u16::from_be_bytes([reply[3], reply[4]]));
    }
    if let Some((index, get, _)) = device.smartshift()? {
        let reply = device.call(index, get, &[])?;
        state.settings.wheel = Some(Wheel::from_bytes(reply[0], reply[1]));
    }
    if let Some(wheel) = device.feature(0x2121)? {
        state.settings.inverted = Some(device.call(wheel, 1, &[])?[0] & INVERT != 0);
    }
    Ok(state)
}

/// getWheelMode's invert bit; the other bits (high resolution, diversion)
/// belong to the kernel driver and are written back unchanged.
const INVERT: u8 = 0x04;

/// Sends each setting the mouse has; `None` ones are left alone.
pub fn write(path: &Path, settings: &Settings) -> io::Result<()> {
    let mut device = Device::open(path)?;
    if let (Some(dpi), Some(index)) = (settings.dpi, device.feature(0x2201)?) {
        let [high, low] = dpi.to_be_bytes();
        device.call(index, 3, &[0, high, low])?;
    }
    if let Some(wheel) = settings.wheel
        && let Some((index, _, set)) = device.smartshift()?
    {
        device.call(index, set, &wheel.to_bytes())?;
    }
    if let (Some(inverted), Some(index)) = (settings.inverted, device.feature(0x2121)?) {
        let mode = device.call(index, 1, &[])?[0];
        let mode = if inverted {
            mode | INVERT
        } else {
            mode & !INVERT
        };
        device.call(index, 2, &[mode])?;
    }
    Ok(())
}

/// getSensorDpiList: either plain values, or low, 0xE000 | step, high.
fn dpi_range(reply: &[u8]) -> Option<(u16, u16, u16)> {
    let values: Vec<u16> = reply[1..]
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .take_while(|&value| value != 0)
        .collect();
    match values.as_slice() {
        [low, step, high, ..] if step >> 13 == 0b111 => Some((*low, *high, step & 0x1FFF)),
        [first, .., last] => Some((*first, *last, 50)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_range_from_mx_master_3s() {
        // Captured from an MX Master 3S over Bluetooth.
        let reply = [0x00, 0x00, 0xc8, 0xe0, 0x32, 0x1f, 0x40, 0x00, 0x00];
        assert_eq!(dpi_range(&reply), Some((200, 8000, 50)));
        assert_eq!(dpi_range(&[0, 0, 0]), None);
    }

    #[test]
    fn wheel_modes_round_trip() {
        // Bytes read back from an MX Master 3S after each write.
        assert_eq!(Wheel::from_bytes(2, 10), Wheel::SmartShift(10));
        assert_eq!(Wheel::from_bytes(2, 0xFF), Wheel::Ratchet);
        assert_eq!(Wheel::from_bytes(1, 0xFF), Wheel::FreeSpin);
        assert_eq!(Wheel::SmartShift(10).to_bytes(), [2, 10, 0]);
        assert_eq!(Wheel::Ratchet.to_bytes(), [2, 0xFF, 0]);
        assert_eq!(Wheel::FreeSpin.to_bytes(), [1, 0, 0]);
    }

    #[test]
    fn finds_logitech_mice_only() {
        let root = std::env::temp_dir().join(format!("openhub-hidraw-{}", std::process::id()));
        for (node, id, uniq, name) in [
            (
                "hidraw2",
                "0005:000028E9:00002000",
                "aa:bb",
                "YUNZII AL68 BT1",
            ),
            (
                "hidraw3",
                "0005:0000046D:0000B034",
                "D2:8A:11:22:33:44",
                "Logitech MX Master 3S",
            ),
        ] {
            let dir = root.join(node).join("device");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("uevent"),
                format!("HID_ID={id}\nHID_NAME={name}\nHID_UNIQ={uniq}\n"),
            )
            .unwrap();
        }
        let nodes = find_in(&root, Path::new("/dev"));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            nodes,
            vec![Node {
                path: "/dev/hidraw3".into(),
                uniq: "d2:8a:11:22:33:44".into(),
                name: "Logitech MX Master 3S".into(),
            }]
        );
    }
}
