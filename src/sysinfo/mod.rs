//! About this computer: what GNOME Settings › System › About shows, from
//! systemd-hostnamed where it runs, else from `/sys` and `/proc` (the host's,
//! also in the Flatpak). Nothing here changes while Openhub runs, so it is
//! read once.

use std::collections::HashMap;
use std::path::Path;

use gtk::glib::prelude::*;
use gtk::{gio, glib};

use crate::usb::ids::UsbIds;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct About {
    pub name: String,
    pub model: String,
    /// The chassis icon from hostnamed, such as `computer-laptop`.
    pub icon: String,
    /// os-release's `LOGO`, such as `fedora-logo-icon`.
    pub logo: Option<String>,
    pub processor: Option<String>,
    pub memory: Option<String>,
    pub graphics: Vec<String>,
    pub disk: Option<String>,
    pub firmware: Option<String>,
    pub os: String,
    pub os_type: String,
    pub desktop: Option<String>,
    pub kernel: String,
}

impl About {
    /// (title, value) rows for the Hardware group.
    pub fn hardware(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![("Model", self.model.clone())];
        rows.extend(self.processor.clone().map(|v| ("Processor", v)));
        rows.extend(self.memory.clone().map(|v| ("Memory", v)));
        if !self.graphics.is_empty() {
            rows.push(("Graphics", self.graphics.join(" / ")));
        }
        rows.extend(self.disk.clone().map(|v| ("Disk Capacity", v)));
        rows.extend(self.firmware.clone().map(|v| ("Firmware Version", v)));
        rows.retain(|(_, value)| !value.is_empty());
        rows
    }

    /// (title, value) rows for the Software group; `windowing` comes from
    /// GTK.
    pub fn software(&self, windowing: &str) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("OS Name", self.os.clone()),
            ("OS Type", self.os_type.clone()),
        ];
        rows.extend(self.desktop.clone().map(|v| ("Desktop", v)));
        rows.push(("Windowing System", windowing.to_owned()));
        rows.push(("Kernel Version", self.kernel.clone()));
        rows.retain(|(_, value)| !value.is_empty());
        rows
    }

    /// The text Copy Details puts on the clipboard. No serials or IDs.
    pub fn report(&self, windowing: &str) -> String {
        let mut text = String::new();
        for (heading, rows) in [
            ("Hardware", self.hardware()),
            ("Software", self.software(windowing)),
        ] {
            text.push_str(&format!("{heading}\n"));
            for (title, value) in rows {
                text.push_str(&format!("  {title}: {value}\n"));
            }
        }
        text
    }
}

/// Everything, from the system and session buses where they answer.
pub async fn load() -> About {
    let system = gio::bus_get_future(gio::BusType::System).await.ok();
    let session = gio::bus_get_future(gio::BusType::Session).await.ok();
    let host = match &system {
        Some(bus) => properties(
            bus,
            "org.freedesktop.hostname1",
            "/org/freedesktop/hostname1",
        )
        .await
        .unwrap_or_default(),
        None => HashMap::new(),
    };
    let text = |key: &str| {
        host.get(key)
            .and_then(|v| v.get::<String>())
            .filter(|v| !v.is_empty())
    };
    let os_release = [
        "/run/host/os-release",
        "/etc/os-release",
        "/usr/lib/os-release",
    ]
    .iter()
    .find_map(|path| std::fs::read_to_string(path).ok())
    .unwrap_or_default();
    let dmi = |name: &str| read_trimmed(&Path::new("/sys/class/dmi/id").join(name));

    let model = match (text("HardwareVendor"), text("HardwareModel")) {
        (Some(vendor), Some(model)) => format!("{vendor} {model}"),
        _ => dmi_model(
            dmi("sys_vendor"),
            dmi("product_name"),
            dmi("product_version"),
        ),
    };
    let firmware = text("FirmwareVersion")
        .map(|version| {
            let date = host
                .get("FirmwareDate")
                .and_then(|v| v.get::<u64>())
                .and_then(|usec| glib::DateTime::from_unix_utc((usec / 1_000_000) as i64).ok())
                .and_then(|d| d.format("%Y-%m-%d").ok());
            match date {
                Some(date) => format!("{version} ({date})"),
                None => version,
            }
        })
        .or_else(|| dmi("bios_version"));
    let graphics = match &system {
        Some(bus) => switcheroo_gpus(bus).await,
        None => Vec::new(),
    };
    let graphics = if graphics.is_empty() {
        drm_gpus(Path::new("/sys/class/drm"), UsbIds::pci())
    } else {
        graphics
    };
    let disk = match &system {
        Some(bus) => disk_capacity(bus).await,
        None => None,
    };
    let shell = match &session {
        Some(bus) => properties(bus, "org.gnome.Shell", "/org/gnome/Shell")
            .await
            .and_then(|p| p.get("ShellVersion")?.get::<String>()),
        None => None,
    };
    let kernel = text("KernelRelease")
        .or_else(|| read_trimmed(Path::new("/proc/sys/kernel/osrelease")))
        .unwrap_or_default();

    About {
        name: text("PrettyHostname")
            .or_else(|| text("Hostname"))
            .or_else(|| read_trimmed(Path::new("/etc/hostname")))
            .unwrap_or_default(),
        model,
        icon: text("IconName").unwrap_or_else(|| "computer".to_owned()),
        logo: os_release_value(&os_release, "LOGO"),
        processor: std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|t| processor(&t)),
        memory: installed_memory(Path::new("/run/udev/data/+dmi:id")).or_else(|| {
            std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|t| memory_total(&t))
        }),
        graphics,
        disk,
        firmware,
        os: text("OperatingSystemPrettyName")
            .or_else(|| os_release_value(&os_release, "PRETTY_NAME"))
            .unwrap_or_else(|| "Linux".to_owned()),
        os_type: if cfg!(target_pointer_width = "64") {
            "64-bit".to_owned()
        } else {
            "32-bit".to_owned()
        },
        desktop: desktop(std::env::var("XDG_CURRENT_DESKTOP").ok(), shell),
        kernel: format!("Linux {kernel}"),
    }
}

/// "Wayland" or "X11", from the GDK display's type name.
pub fn windowing(display: &gtk::gdk::Display) -> String {
    let name = display.type_().name();
    if name.contains("Wayland") {
        "Wayland".to_owned()
    } else if name.contains("X11") {
        "X11".to_owned()
    } else {
        name.trim_start_matches("Gdk")
            .trim_end_matches("Display")
            .to_owned()
    }
}

async fn properties(
    bus: &gio::DBusConnection,
    name: &str,
    path: &str,
) -> Option<HashMap<String, glib::Variant>> {
    let interface = name;
    let reply = bus
        .call_future(
            Some(name),
            path,
            "org.freedesktop.DBus.Properties",
            "GetAll",
            Some(&(interface,).to_variant()),
            Some(glib::VariantTy::new("(a{sv})").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await
        .ok()?;
    reply.child_value(0).get()
}

/// Every GPU's name from switcheroo-control, where it runs.
async fn switcheroo_gpus(bus: &gio::DBusConnection) -> Vec<String> {
    let Some(props) = properties(
        bus,
        "net.hadess.SwitcherooControl",
        "/net/hadess/SwitcherooControl",
    )
    .await
    else {
        return Vec::new();
    };
    let Some(gpus) = props.get("GPUs") else {
        return Vec::new();
    };
    (0..gpus.n_children())
        .filter_map(|i| {
            glib::VariantDict::new(Some(&gpus.child_value(i)))
                .lookup_value("Name", None)?
                .get::<String>()
        })
        .collect()
}

/// The total size of the built-in drives (not removable ones), from UDisks2.
async fn disk_capacity(bus: &gio::DBusConnection) -> Option<String> {
    let reply = bus
        .call_future(
            Some("org.freedesktop.UDisks2"),
            "/org/freedesktop/UDisks2",
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
            Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()),
            gio::DBusCallFlags::NONE,
            5000,
        )
        .await
        .ok()?;
    let list = reply.child_value(0);
    let mut total = 0u64;
    for i in 0..list.n_children() {
        // a{sa{sv}}: each interface's name and properties.
        let interfaces = list.child_value(i).child_value(1);
        let Some(drive) = (0..interfaces.n_children())
            .map(|j| interfaces.child_value(j))
            .find(|entry| entry.child_value(0).str() == Some("org.freedesktop.UDisks2.Drive"))
        else {
            continue;
        };
        let drive = glib::VariantDict::new(Some(&drive.child_value(1)));
        let get_bool = |key| drive.lookup_value(key, None).and_then(|v| v.get::<bool>());
        if get_bool("Removable") == Some(true) || get_bool("Ejectable") == Some(true) {
            continue;
        }
        total += drive
            .lookup_value("Size", None)
            .and_then(|v| v.get::<u64>())
            .unwrap_or(0);
    }
    (total > 0).then(|| glib::format_size(total).to_string())
}

fn read_trimmed(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// The model from DMI. Lenovo keeps the marketing name in
/// `product_version` and a part number in `product_name`, as systemd knows.
fn dmi_model(vendor: Option<String>, name: Option<String>, version: Option<String>) -> String {
    let vendor = vendor.unwrap_or_default();
    let name = if vendor.eq_ignore_ascii_case("lenovo") {
        version.or(name)
    } else {
        name
    };
    format!("{vendor} {}", name.unwrap_or_default())
        .trim()
        .to_owned()
}

fn os_release_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix('=')?;
        let value = value.trim().trim_matches('"').trim_matches('\'');
        (!value.is_empty()).then(|| value.to_owned())
    })
}

/// "13th Gen Intel® Core™ i5-1335U × 12": the first model name, and the
/// number of logical processors.
fn processor(cpuinfo: &str) -> Option<String> {
    // ponytail: ARM's cpuinfo has no "model name"; the row is hidden there
    // until /sys/devices/soc0 is read.
    let name = cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "model name").then(|| value.trim())
    })?;
    let threads = cpuinfo
        .lines()
        .filter(|line| line.split(':').next().map(str::trim) == Some("processor"))
        .count();
    let name = name.replace("(R)", "®").replace("(TM)", "™");
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(if threads > 1 {
        format!("{name} × {threads}")
    } else {
        name
    })
}

/// The installed memory from the firmware's table, as udev records it
/// (`MEMORY_DEVICE_0_SIZE=…`). Not readable in the Flatpak.
fn installed_memory(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let total: u64 = text
        .lines()
        .filter_map(|line| {
            let line = line.strip_prefix("E:MEMORY_DEVICE_")?;
            let (key, value) = line.split_once('=')?;
            // MEMORY_DEVICE_0_SIZE, not _VOLATILE_SIZE and the like.
            let (index, field) = key.split_once('_')?;
            (index.parse::<u32>().is_ok() && field == "SIZE").then(|| value.parse::<u64>().ok())?
        })
        .sum();
    (total > 0).then(|| iec(total))
}

/// `MemTotal` rounded up to a whole GiB, since the kernel keeps some back.
fn memory_total(meminfo: &str) -> Option<String> {
    let kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    const GIB: u64 = 1 << 30;
    Some(iec((kib * 1024).div_ceil(GIB) * GIB))
}

fn iec(bytes: u64) -> String {
    glib::format_size_full(bytes, glib::FormatSizeFlags::IEC_UNITS).to_string()
}

/// GPUs from `cardN/device/{vendor,device}`, named from `pci.ids`.
fn drm_gpus(drm: &Path, ids: &UsbIds) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards: Vec<_> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.starts_with("card") && !name.contains('-')
        })
        .map(|e| e.path())
        .collect();
    cards.sort();
    cards
        .iter()
        .filter_map(|card| {
            let id = |file: &str| {
                let text = read_trimmed(&card.join("device").join(file))?;
                u16::from_str_radix(text.trim_start_matches("0x"), 16).ok()
            };
            let (vendor, device) = (id("vendor")?, id("device")?);
            Some(match (ids.vendor(vendor), ids.product(vendor, device)) {
                (Some(v), Some(d)) => format!("{v} {d}"),
                (Some(v), None) => format!("{v} {device:04x}"),
                _ => format!("{vendor:04x}:{device:04x}"),
            })
        })
        .collect()
}

/// "GNOME 50.6", or the desktop alone without its version.
fn desktop(current: Option<String>, shell: Option<String>) -> Option<String> {
    let name = current
        .as_deref()
        .and_then(|c| c.split(':').next_back())
        .map(str::to_owned);
    match (name, shell) {
        (_, Some(version)) => Some(format!("GNOME {version}")),
        (Some(name), None) if !name.is_empty() => Some(name),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_processor() {
        let cpuinfo = "processor\t: 0\nmodel name\t: 13th Gen Intel(R) Core(TM) i5-1335U\n\n\
                       processor\t: 1\nmodel name\t: 13th Gen Intel(R) Core(TM) i5-1335U\n";
        assert_eq!(
            processor(cpuinfo).as_deref(),
            Some("13th Gen Intel® Core™ i5-1335U × 2")
        );
        assert_eq!(processor("Hardware\t: BCM2835\n"), None);
    }

    #[test]
    fn rounds_memory_to_the_installed_size() {
        assert_eq!(
            memory_total("MemTotal:       24248624 kB\nMemFree: 1 kB\n").as_deref(),
            Some("24.0\u{a0}GiB")
        );
        let root = std::env::temp_dir().join(format!("openhub-dmi-{}", std::process::id()));
        std::fs::write(
            &root,
            "E:MEMORY_ARRAY_NUM_DEVICES=2\nE:MEMORY_DEVICE_0_SIZE=17179869184\n\
             E:MEMORY_DEVICE_0_VOLATILE_SIZE=17179869184\nE:MEMORY_DEVICE_1_SIZE=8589934592\n",
        )
        .unwrap();
        let found = installed_memory(&root);
        std::fs::remove_file(&root).unwrap();
        assert_eq!(found.as_deref(), Some("24.0\u{a0}GiB"));
    }

    #[test]
    fn reads_os_release_and_dmi() {
        let text = "NAME=\"Fedora Linux\"\nPRETTY_NAME=\"Fedora Linux 44 (Workstation Edition)\"\nLOGO=fedora-logo-icon\n";
        assert_eq!(
            os_release_value(text, "PRETTY_NAME").as_deref(),
            Some("Fedora Linux 44 (Workstation Edition)")
        );
        assert_eq!(
            os_release_value(text, "LOGO").as_deref(),
            Some("fedora-logo-icon")
        );
        assert_eq!(os_release_value(text, "NAM"), None);
        let lenovo = dmi_model(
            Some("LENOVO".into()),
            Some("21JKS00U00".into()),
            Some("ThinkPad E14 Gen 5".into()),
        );
        assert_eq!(lenovo, "LENOVO ThinkPad E14 Gen 5");
        assert_eq!(
            dmi_model(Some("Dell Inc.".into()), Some("XPS 13 9340".into()), None),
            "Dell Inc. XPS 13 9340"
        );
    }

    #[test]
    fn names_gpus_from_pci_ids() {
        let root = std::env::temp_dir().join(format!("openhub-gpu-{}", std::process::id()));
        let device = root.join("card1/device");
        std::fs::create_dir_all(&device).unwrap();
        std::fs::create_dir_all(root.join("card1-eDP-1")).unwrap();
        std::fs::write(device.join("vendor"), "0x8086\n").unwrap();
        std::fs::write(device.join("device"), "0xa7a1\n").unwrap();
        let ids =
            UsbIds::parse("8086  Intel Corporation\n\ta7a1  Raptor Lake-P [Iris Xe Graphics]\n");
        let found = drm_gpus(&root, &ids);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            found,
            ["Intel Corporation Raptor Lake-P [Iris Xe Graphics]"]
        );
    }

    #[test]
    fn names_the_desktop() {
        assert_eq!(
            desktop(Some("GNOME".into()), Some("50.6".into())).as_deref(),
            Some("GNOME 50.6")
        );
        assert_eq!(
            desktop(Some("ubuntu:GNOME".into()), None).as_deref(),
            Some("GNOME")
        );
        assert_eq!(desktop(None, None), None);
    }

    #[test]
    fn copies_details_without_empty_rows() {
        let about = About {
            model: "Lenovo ThinkPad E14 Gen 5".into(),
            processor: Some("CPU × 12".into()),
            os: "Fedora Linux 44".into(),
            os_type: "64-bit".into(),
            kernel: "Linux 7.2".into(),
            ..About::default()
        };
        assert_eq!(
            about.report("Wayland"),
            "Hardware\n  Model: Lenovo ThinkPad E14 Gen 5\n  Processor: CPU × 12\n\
             Software\n  OS Name: Fedora Linux 44\n  OS Type: 64-bit\n  \
             Windowing System: Wayland\n  Kernel Version: Linux 7.2\n"
        );
    }
}
