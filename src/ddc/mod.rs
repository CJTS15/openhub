//! External monitor brightness over DDC/CI: MCCS's Get and Set VCP Feature
//! for VCP code 0x10, at I2C address 0x37, on the bus the monitor's DRM
//! connector links to (`/sys/class/drm/cardN-DP-2/ddc`), so no bus is
//! probed. Calls block for tens of milliseconds; run them off the main
//! thread.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::ioctl::{IntegerSetter, ioctl};

/// The monitor's DDC/CI address, and the host's as MCCS writes them.
const ADDRESS: u8 = 0x37;
const DESTINATION: u8 = 0x6E;
const SOURCE: u8 = 0x51;
pub const BRIGHTNESS: u8 = 0x10;
/// `I2C_SLAVE`: the address following reads and writes go to.
const I2C_SLAVE: rustix::ioctl::Opcode = 0x0703;
/// MCCS asks for at least 40 ms between a request and its reply.
const PAUSE: Duration = Duration::from_millis(50);

/// The I2C device for a connector such as `DP-2`, if its DRM connector has
/// a DDC bus.
pub fn bus_for(connector: &str) -> Option<PathBuf> {
    bus_in(Path::new("/sys/class/drm"), connector)
}

fn bus_in(drm: &Path, connector: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(drm).ok()?;
    entries.flatten().find_map(|entry| {
        let file = entry.file_name();
        let (_, name) = file.to_str()?.split_once('-')?;
        // HDMI connectors are "HDMI-A-1" in sysfs and "HDMI-1" to Mutter.
        let matches = name == connector || name.replace("-A-", "-") == connector;
        if !matches {
            return None;
        }
        let bus = std::fs::read_link(entry.path().join("ddc")).ok()?;
        let bus = bus.file_name()?.to_str()?.to_owned();
        bus.starts_with("i2c-")
            .then(|| PathBuf::from("/dev").join(bus))
    })
}

pub struct Monitor {
    file: File,
}

impl Monitor {
    pub fn open(bus: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(bus)?;
        // SAFETY: I2C_SLAVE takes the 7-bit address as an integer.
        unsafe {
            ioctl(
                &file,
                IntegerSetter::<I2C_SLAVE>::new_usize(usize::from(ADDRESS)),
            )?
        };
        Ok(Self { file })
    }

    /// The current and maximum value of a VCP feature.
    pub fn get(&mut self, code: u8) -> io::Result<(u16, u16)> {
        self.file
            .write_all(&with_checksum(&[SOURCE, 0x82, 0x01, code]))?;
        std::thread::sleep(PAUSE);
        let mut reply = [0u8; 11];
        self.file.read_exact(&mut reply)?;
        parse_reply(&reply, code)
    }

    pub fn set(&mut self, code: u8, value: u16) -> io::Result<()> {
        let [high, low] = value.to_be_bytes();
        self.file
            .write_all(&with_checksum(&[SOURCE, 0x84, 0x03, code, high, low]))?;
        // The monitor needs the pause before it takes the next command.
        std::thread::sleep(PAUSE);
        Ok(())
    }
}

/// A DDC/CI message with its checksum: the XOR of the destination address
/// and every byte.
fn with_checksum(bytes: &[u8]) -> Vec<u8> {
    let checksum = bytes.iter().fold(DESTINATION, |sum, b| sum ^ b);
    let mut message = bytes.to_vec();
    message.push(checksum);
    message
}

/// A Get VCP Feature reply: source, length, 0x02, result, code, type,
/// maximum and current value (big-endian), checksum (with 0x50 as the
/// virtual source).
fn parse_reply(reply: &[u8; 11], code: u8) -> io::Result<(u16, u16)> {
    let checksum = reply[..10].iter().fold(0x50, |sum, b| sum ^ b);
    if checksum != reply[10] || reply[2] != 0x02 || reply[4] != code {
        return Err(io::Error::other("The monitor's reply was garbled"));
    }
    if reply[3] != 0 {
        return Err(io::Error::other("The monitor doesn't support this setting"));
    }
    let max = u16::from_be_bytes([reply[6], reply[7]]);
    let current = u16::from_be_bytes([reply[8], reply[9]]);
    Ok((current, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_carry_mccs_checksums() {
        // Get VCP 0x10, as in the MCCS standard's example.
        assert_eq!(
            with_checksum(&[0x51, 0x82, 0x01, 0x10]),
            [0x51, 0x82, 0x01, 0x10, 0xAC]
        );
        // Set brightness to 50.
        assert_eq!(
            with_checksum(&[0x51, 0x84, 0x03, 0x10, 0x00, 0x32]),
            [0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x9A]
        );
    }

    #[test]
    fn parses_a_brightness_reply() {
        // Brightness 35 of 100.
        let mut reply = [
            0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x23, 0x00,
        ];
        reply[10] = reply[..10].iter().fold(0x50, |sum, b| sum ^ b);
        assert_eq!(parse_reply(&reply, BRIGHTNESS).unwrap(), (35, 100));
        // Unsupported, garbled, or for another code.
        let mut unsupported = reply;
        unsupported[3] = 1;
        unsupported[10] = unsupported[..10].iter().fold(0x50, |sum, b| sum ^ b);
        assert!(parse_reply(&unsupported, BRIGHTNESS).is_err());
        let mut garbled = reply;
        garbled[9] = 0x24;
        assert!(parse_reply(&garbled, BRIGHTNESS).is_err());
        assert!(parse_reply(&reply, 0x12).is_err());
    }

    #[test]
    fn finds_a_connectors_bus() {
        let root = std::env::temp_dir().join(format!("openhub-ddc-{}", std::process::id()));
        for (connector, bus) in [("card1-DP-2", "i2c-13"), ("card1-HDMI-A-1", "i2c-5")] {
            let dir = root.join(connector);
            std::fs::create_dir_all(dir.join(bus)).unwrap();
            std::os::unix::fs::symlink(dir.join(bus), dir.join("ddc")).unwrap();
        }
        std::fs::create_dir_all(root.join("card1-eDP-1")).unwrap();
        let found = (
            bus_in(&root, "DP-2"),
            bus_in(&root, "HDMI-1"),
            bus_in(&root, "eDP-1"),
        );
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found.0, Some(PathBuf::from("/dev/i2c-13")));
        assert_eq!(found.1, Some(PathBuf::from("/dev/i2c-5")));
        assert_eq!(found.2, None);
    }
}
