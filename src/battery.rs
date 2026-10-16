//! Batteries of USB devices that UPower doesn't list: the kernel's own
//! power supplies (UPower can miss a dongle plugged into a new port, as with
//! a VXE mouse) and Razer mice, which report theirs only through their own
//! protocol. Pages use these where UPower has nothing for the device.

use std::path::Path;

use crate::razer;
use crate::upower::convert::usb_owner;

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// The USB device, such as `3-1.3`.
    pub usb_device: String,
    pub name: String,
    /// 0 to 100.
    pub level: u8,
    /// Read through Razer's protocol: a mouse.
    pub razer: bool,
}

impl Battery {
    /// The (level text, fraction) the pages show, such as ("25%", 0.25).
    pub fn text(&self) -> (String, f64) {
        (format!("{}%", self.level), f64::from(self.level) / 100.0)
    }
}

pub fn usb() -> Vec<Battery> {
    let mut found = kernel(Path::new("/sys/class/power_supply"));
    for node in razer::find() {
        if let Some(level) = razer::battery(&node) {
            found.push(Battery {
                usb_device: node.usb_device,
                name: node.name,
                level,
                razer: true,
            });
        }
    }
    found
}

/// Device batteries (not the laptop's own) under a USB device.
fn kernel(root: &Path) -> Vec<Battery> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let attr = |name: &str| {
                std::fs::read_to_string(path.join(name))
                    .ok()
                    .map(|value| value.trim().to_owned())
            };
            if attr("scope")? != "Device" || attr("type")? != "Battery" {
                return None;
            }
            let usb_device = usb_owner(&std::fs::canonicalize(&path).ok()?)?;
            Some(Battery {
                usb_device,
                name: attr("model_name").unwrap_or_default(),
                level: attr("capacity")?.parse().ok()?,
                razer: false,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_device_batteries_under_usb() {
        let root = std::env::temp_dir().join(format!("openhub-power-{}", std::process::id()));
        // The VXE dongle's battery, as the kysona driver lays it out.
        let vxe = root.join(
            "devices/pci0000:00/0000:00:14.0/usb3/3-1/3-1.3/3-1.3:1.1/0003:3554:F58A.0011/\
             power_supply/kysona-battery",
        );
        let laptop = root.join("devices/LNXSYSTM:00/PNP0C0A:00/power_supply/BAT0");
        for (dir, scope, capacity) in [(&vxe, "Device", "25"), (&laptop, "System", "80")] {
            std::fs::create_dir_all(dir).unwrap();
            for (name, value) in [
                ("scope", scope),
                ("type", "Battery"),
                ("capacity", capacity),
                ("model_name", "Compx VXE NordicMouse 1K Dongle"),
            ] {
                std::fs::write(dir.join(name), value).unwrap();
            }
        }
        let class = root.join("class");
        std::fs::create_dir_all(&class).unwrap();
        std::os::unix::fs::symlink(&vxe, class.join("kysona-battery")).unwrap();
        std::os::unix::fs::symlink(&laptop, class.join("BAT0")).unwrap();
        let found = kernel(&class);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            found,
            vec![Battery {
                usb_device: "3-1.3".into(),
                name: "Compx VXE NordicMouse 1K Dongle".into(),
                level: 25,
                razer: false,
            }]
        );
        assert_eq!(found[0].text(), ("25%".to_owned(), 0.25));
    }
}
