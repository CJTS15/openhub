//! VIA keyboards over `/dev/hidraw*`: QMK's raw HID protocol for lighting
//! and the keymap stored on the keyboard. Only over the USB cable; wireless
//! links don't carry the raw HID interface. Calls block briefly; run them
//! off the main thread.

pub mod definition;
pub mod keycodes;

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

pub use definition::Definition;

/// A keyboard's VIA interface, from sysfs.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub path: PathBuf,
    pub vendor: u16,
    pub product: u16,
    /// The USB device, such as `3-1.4`; matches `InputDevice::usb_device`.
    pub usb_device: String,
    /// Bytes per report: 32 normally, 64 on the Yunzii AL68.
    pub report_size: usize,
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
            let descriptor = std::fs::read(device.join("report_descriptor")).ok()?;
            let report_size = raw_report_size(&descriptor)?;
            let uevent = std::fs::read_to_string(device.join("uevent")).ok()?;
            // HID_ID=0003:00004D4B:0000304E
            let id = uevent
                .lines()
                .find_map(|line| line.strip_prefix("HID_ID="))?;
            let mut parts = id.split(':');
            let bus = parts.next()?;
            let vendor = u16::from_str_radix(parts.next()?.trim_start_matches("0000"), 16).ok()?;
            let product = u16::from_str_radix(parts.next()?.trim_start_matches("0000"), 16).ok()?;
            if bus != "0003" {
                return None;
            }
            // …/3-1.4/3-1.4:1.1/0003:4D4B:304E.0012: the parent is the interface.
            let resolved = std::fs::canonicalize(&device).ok()?;
            let interface = resolved.parent()?.file_name()?.to_str()?;
            let (usb_device, _) = interface.split_once(':')?;
            Some(Node {
                path: dev.join(entry.file_name()),
                vendor,
                product,
                usb_device: usb_device.to_owned(),
                report_size,
            })
        })
        .collect();
    nodes.sort_by(|a, b| a.path.cmp(&b.path));
    nodes
}

/// The report size of VIA's raw HID collection (usage page 0xFF60, usage
/// 0x61), or `None` if the descriptor doesn't have one. A small walk over
/// the descriptor's short items, enough for the collection QMK writes.
fn raw_report_size(descriptor: &[u8]) -> Option<usize> {
    let mut at = 0;
    let mut usage_page = 0u32;
    let mut in_raw = false;
    while at < descriptor.len() {
        let prefix = descriptor[at];
        let size = match prefix & 0x03 {
            3 => 4,
            n => usize::from(n),
        };
        let data = descriptor.get(at + 1..at + 1 + size)?;
        let value = data
            .iter()
            .rev()
            .fold(0u32, |value, byte| value << 8 | u32::from(*byte));
        match prefix & 0xFC {
            0x04 => usage_page = value, // Usage Page
            0x08 if usage_page == 0xFF60 && value == 0x61 => in_raw = true, // Usage
            0x94 if in_raw => return usize::try_from(value).ok(), // Report Count
            _ => {}
        }
        at += 1 + size;
    }
    None
}

pub struct Device {
    file: File,
    report_size: usize,
}

impl Device {
    pub fn open(node: &Node) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(&node.path)?;
        Ok(Self {
            file,
            report_size: node.report_size,
        })
    }

    /// Sends a command and returns the reply, which starts with the command.
    fn call(&mut self, request: &[u8]) -> io::Result<Vec<u8>> {
        let mut report = vec![0u8; self.report_size + 1]; // Report ID 0 first.
        report[1..=request.len()].copy_from_slice(request);
        self.file.write_all(&report)?;
        let fd = self.file.as_fd();
        let mut fds = [rustix::event::PollFd::new(
            &fd,
            rustix::event::PollFlags::IN,
        )];
        let timeout = rustix::event::Timespec {
            tv_sec: 1,
            tv_nsec: 0,
        };
        if rustix::event::poll(&mut fds, Some(&timeout))? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "The keyboard didn't answer",
            ));
        }
        let mut reply = vec![0u8; self.report_size];
        let length = self.file.read(&mut reply)?;
        reply.truncate(length);
        // 0xFF: the firmware doesn't know the command.
        match reply.first() {
            Some(&command) if command == request[0] => Ok(reply),
            _ => Err(io::Error::other("The keyboard doesn't support this")),
        }
    }

    pub fn keycode(&mut self, layer: u8, row: u8, col: u8) -> io::Result<u16> {
        let reply = self.call(&[0x04, layer, row, col])?;
        Ok(u16::from_be_bytes([reply[4], reply[5]]))
    }

    /// Writes a key to the keymap stored on the keyboard.
    pub fn set_keycode(&mut self, layer: u8, row: u8, col: u8, code: u16) -> io::Result<()> {
        let [high, low] = code.to_be_bytes();
        self.call(&[0x05, layer, row, col, high, low]).map(drop)
    }

    /// Puts every key back to the firmware's own keymap.
    pub fn reset_keymap(&mut self) -> io::Result<()> {
        self.call(&[0x06]).map(drop)
    }

    /// A lighting or other custom value: `len` bytes after the channel and ID.
    pub fn value(&mut self, channel: u8, id: u8, len: usize) -> io::Result<Vec<u8>> {
        let reply = self.call(&[0x08, channel, id])?;
        Ok(reply[3..3 + len].to_vec())
    }

    /// Sets a value live; `save` keeps it after unplugging.
    pub fn set_value(&mut self, channel: u8, id: u8, data: &[u8]) -> io::Result<()> {
        let mut request = vec![0x07, channel, id];
        request.extend_from_slice(data);
        self.call(&request).map(drop)
    }

    pub fn save(&mut self, channel: u8) -> io::Result<()> {
        self.call(&[0x09, channel]).map(drop)
    }
}

/// Saved definitions live per USB ID, imported once by the user.
pub fn definition_path(vendor: u16, product: u16) -> PathBuf {
    gtk::glib::user_config_dir().join(format!("openhub/via/{vendor:04x}-{product:04x}.json"))
}

pub fn load_definition(vendor: u16, product: u16) -> Option<Definition> {
    let text = std::fs::read_to_string(definition_path(vendor, product)).ok()?;
    Definition::parse(&text).ok()
}

/// Checks `text` is a definition for this keyboard, then keeps it.
pub fn import_definition(vendor: u16, product: u16, text: &str) -> Result<Definition, String> {
    let definition = Definition::parse(text)?;
    if (definition.vendor, definition.product) != (vendor, product) {
        return Err(format!(
            "This file is for {} ({:04X}:{:04X}), not this keyboard ({vendor:04X}:{product:04X})",
            definition.name, definition.vendor, definition.product
        ));
    }
    let path = definition_path(vendor, product);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not save the definition: {e}"))?;
    }
    gtk::glib::file_set_contents(&path, text.as_bytes())
        .map_err(|e| format!("Could not save the definition: {e}"))?;
    Ok(definition)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_size_from_the_al68_descriptor() {
        // The Yunzii AL68's raw HID interface, from sysfs.
        let al68 = [
            0x06, 0x60, 0xff, 0x09, 0x61, 0xa1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xff, 0x00,
            0x95, 0x40, 0x75, 0x08, 0x81, 0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xff, 0x00, 0x95,
            0x40, 0x75, 0x08, 0x91, 0x02, 0xc0,
        ];
        assert_eq!(raw_report_size(&al68), Some(64));
        // A boot keyboard interface has no raw HID collection.
        let keyboard = [0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x95, 0x08, 0xc0];
        assert_eq!(raw_report_size(&keyboard), None);
    }
}
