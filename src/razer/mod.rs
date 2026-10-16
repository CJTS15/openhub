//! Razer mice over `/dev/hidraw*`: 90-byte control reports sent as HID
//! feature reports, the protocol OpenRazer's kernel driver uses, so neither
//! its module nor its daemon is needed. Razer mice store DPI themselves
//! (tested on an Orochi V2: it survived switching off), so nothing needs
//! restoring when they reconnect. Calls block briefly; run them off the
//! main thread.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::ioctl::{Updater, ioctl, opcode};

const RAZER: u32 = 0x1532;
/// Report ID 0, then the 90-byte report.
type Buffer = [u8; 91];
const SET_FEATURE: rustix::ioctl::Opcode = opcode::read_write::<Buffer>(b'H', 0x06);
const GET_FEATURE: rustix::ioctl::Opcode = opcode::read_write::<Buffer>(b'H', 0x07);
/// Store in the mouse's memory, not only until it's switched off.
const VARSTORE: u8 = 0x01;

/// A Razer mouse's control interface (USB interface 0), from sysfs.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub path: PathBuf,
    /// The USB device, such as `3-3`; matches `InputDevice::usb_device`.
    pub usb_device: String,
    pub product: u16,
}

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
            let device = entry.path().join("device");
            let uevent = std::fs::read_to_string(device.join("uevent")).ok()?;
            // HID_ID=0003:00001532:00000094
            let id = uevent
                .lines()
                .find_map(|line| line.strip_prefix("HID_ID="))?;
            let mut parts = id.split(':');
            let bus = parts.next()?;
            let vendor = u32::from_str_radix(parts.next()?, 16).ok()?;
            let product = u16::from_str_radix(parts.next()?, 16).ok()?;
            if bus != "0003" || vendor != RAZER {
                return None;
            }
            // …/3-3/3-3:1.0/0003:1532:0094.0005: the parent is the interface.
            let resolved = std::fs::canonicalize(&device).ok()?;
            let interface = resolved.parent()?.file_name()?.to_str()?;
            let (usb_device, number) = interface.split_once(':')?;
            (number == "1.0").then(|| Node {
                path: dev.join(entry.file_name()),
                usb_device: usb_device.to_owned(),
                product,
            })
        })
        .collect();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    nodes
}

/// A Razer mouse's current settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct State {
    pub dpi: u16,
    pub max_dpi: u16,
    /// Reports per second: 125, 500 or 1000.
    pub poll_rate: Option<u16>,
    pub battery: Option<u8>,
}

/// The highest DPI, from OpenRazer's device list; the protocol can't ask.
fn max_dpi(product: u16) -> u16 {
    match product {
        // Orochi V2 on its dongle and over Bluetooth.
        0x0094 | 0x0095 => 18000,
        // ponytail: unlisted models get a common ceiling; add them from
        // OpenRazer's hardware/mouse.py as people report them.
        _ => 16000,
    }
}

/// One 90-byte report: status, transaction ID, size, command class and ID,
/// up to 80 argument bytes, then the XOR of bytes 2 to 87.
fn report(transaction: u8, class: u8, command: u8, size: u8, args: &[u8]) -> Buffer {
    let mut buffer = [0u8; 91];
    let report = &mut buffer[1..];
    report[1] = transaction;
    report[5] = size;
    report[6] = class;
    report[7] = command;
    report[8..8 + args.len()].copy_from_slice(args);
    report[88] = report[2..88].iter().fold(0, |crc, byte| crc ^ byte);
    buffer
}

struct Device {
    file: File,
    transaction: u8,
}

impl Device {
    /// Opens the node and finds the transaction ID this model answers to;
    /// OpenRazer lists 0x1F for the Orochi V2, 0x3F and 0xFF for others.
    fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut device = Device {
            file,
            transaction: 0x1F,
        };
        for transaction in [0x1F, 0x3F, 0xFF] {
            device.transaction = transaction;
            // The firmware version: every model answers it.
            if device.call(0x00, 0x81, 0x02, &[]).is_ok() {
                return Ok(device);
            }
        }
        Err(io::Error::other("This Razer mouse doesn't answer"))
    }

    /// Sends a command and returns the reply's argument bytes.
    fn call(&mut self, class: u8, command: u8, size: u8, args: &[u8]) -> io::Result<[u8; 80]> {
        let mut buffer = report(self.transaction, class, command, size, args);
        // SAFETY: HIDIOCSFEATURE and HIDIOCGFEATURE take the buffer, sized in
        // the opcode, and read or fill it.
        unsafe { ioctl(&self.file, Updater::<SET_FEATURE, Buffer>::new(&mut buffer))? };
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(50));
            let mut reply = [0u8; 91];
            unsafe { ioctl(&self.file, Updater::<GET_FEATURE, Buffer>::new(&mut reply))? };
            let reply = &reply[1..];
            match reply[0] {
                0x01 => continue, // Busy.
                0x02 if reply[6] == class && reply[7] == command => {
                    let mut out = [0u8; 80];
                    out.copy_from_slice(&reply[8..88]);
                    return Ok(out);
                }
                0x05 => return Err(io::Error::other("Not supported by this mouse")),
                status => {
                    return Err(io::Error::other(format!(
                        "The mouse refused the command ({status:#04x})"
                    )));
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "The mouse didn't answer. Move it to wake it up.",
        ))
    }
}

pub fn read(node: &Node) -> io::Result<State> {
    let mut device = Device::open(&node.path)?;
    let dpi = device.call(0x04, 0x85, 0x07, &[VARSTORE])?;
    let poll_rate = device
        .call(0x00, 0x85, 0x01, &[])
        .ok()
        .and_then(|reply| poll_rate(reply[0]));
    // 0 to 255; wired mice don't answer.
    let battery = device
        .call(0x07, 0x80, 0x02, &[])
        .ok()
        .map(|reply| (u16::from(reply[1]) * 100 / 255) as u8);
    Ok(State {
        dpi: u16::from_be_bytes([dpi[1], dpi[2]]),
        max_dpi: max_dpi(node.product),
        poll_rate,
        battery,
    })
}

/// Sets DPI (the same on both axes, stored on the mouse) and, if given, the
/// polling rate.
pub fn write(node: &Node, dpi: u16, rate: Option<u16>) -> io::Result<()> {
    let mut device = Device::open(&node.path)?;
    let [high, low] = dpi.to_be_bytes();
    device.call(0x04, 0x05, 0x07, &[VARSTORE, high, low, high, low])?;
    if let Some(byte) = rate.and_then(poll_rate_byte) {
        device.call(0x00, 0x05, 0x01, &[byte])?;
    }
    Ok(())
}

fn poll_rate(byte: u8) -> Option<u16> {
    match byte {
        0x01 => Some(1000),
        0x02 => Some(500),
        0x08 => Some(125),
        _ => None,
    }
}

fn poll_rate_byte(rate: u16) -> Option<u8> {
    match rate {
        1000 => Some(0x01),
        500 => Some(0x02),
        125 => Some(0x08),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_carry_their_checksum() {
        // Get DPI, as sent to an Orochi V2.
        let buffer = report(0x1F, 0x04, 0x85, 0x07, &[VARSTORE]);
        assert_eq!(buffer[0], 0); // Report ID.
        assert_eq!(
            &buffer[1..10],
            &[0x00, 0x1F, 0, 0, 0, 0x07, 0x04, 0x85, 0x01]
        );
        assert_eq!(buffer[89], 0x07 ^ 0x04 ^ 0x85 ^ 0x01);
    }

    #[test]
    fn poll_rates_round_trip() {
        for rate in [125, 500, 1000] {
            assert_eq!(poll_rate_byte(rate).and_then(poll_rate), Some(rate));
        }
        assert_eq!(poll_rate(0x04), None);
        assert_eq!(poll_rate_byte(250), None);
    }

    #[test]
    fn finds_the_control_interface_only() {
        let root = std::env::temp_dir().join(format!("openhub-razer-{}", std::process::id()));
        let usb = root.join("usb/3-3");
        for (node, interface) in [("hidraw5", "1.0"), ("hidraw6", "1.1")] {
            let device = usb.join(format!("3-3:{interface}/0003:1532:0094.0005"));
            std::fs::create_dir_all(&device).unwrap();
            std::fs::write(device.join("uevent"), "HID_ID=0003:00001532:00000094\n").unwrap();
            let class = root.join("class").join(node);
            std::fs::create_dir_all(&class).unwrap();
            std::os::unix::fs::symlink(&device, class.join("device")).unwrap();
        }
        let nodes = find_in(&root.join("class"), Path::new("/dev"));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            nodes,
            vec![Node {
                path: "/dev/hidraw5".into(),
                usb_device: "3-3".into(),
                product: 0x0094,
            }]
        );
        assert_eq!(max_dpi(0x0094), 18000);
    }
}
