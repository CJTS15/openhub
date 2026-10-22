//! Vendor and product names from the `usb.ids` database, for devices whose
//! firmware doesn't name itself.

use std::collections::HashMap;
use std::fs;
use std::sync::OnceLock;

/// Bundled in the Flatpak first, then the usual distro locations.
const PATHS: [&str; 3] = [
    "/app/share/hwdata/usb.ids",
    "/usr/share/hwdata/usb.ids",
    "/usr/share/misc/usb.ids",
];

#[derive(Debug, Default)]
pub struct UsbIds {
    vendors: HashMap<u16, Vendor>,
}

#[derive(Debug)]
struct Vendor {
    name: String,
    products: HashMap<u16, String>,
}

impl UsbIds {
    /// The system database, read on first use; empty when there is none.
    pub fn system() -> &'static Self {
        static IDS: OnceLock<UsbIds> = OnceLock::new();
        IDS.get_or_init(|| {
            PATHS
                .iter()
                .find_map(|path| fs::read(path).ok())
                .map(|bytes| Self::parse(&String::from_utf8_lossy(&bytes)))
                .unwrap_or_default()
        })
    }

    /// `pci.ids`, which has the same layout, from the same places.
    pub fn pci() -> &'static Self {
        static IDS: OnceLock<UsbIds> = OnceLock::new();
        IDS.get_or_init(|| {
            PATHS
                .iter()
                .find_map(|path| fs::read(path.replace("usb.ids", "pci.ids")).ok())
                .map(|bytes| Self::parse(&String::from_utf8_lossy(&bytes)))
                .unwrap_or_default()
        })
    }

    /// Parses the vendor list: `vvvv  Vendor` lines, each followed by
    /// tab-indented `pppp  Product` lines. Later sections (`C 00  Class`) end it.
    pub fn parse(text: &str) -> Self {
        let mut vendors = HashMap::new();
        let mut current = None;
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(product) = line.strip_prefix('\t') {
                if let (Some(vendor), Some((id, name))) = (current, entry(product)) {
                    let vendor: &mut Vendor = vendors.get_mut(&vendor).expect("current vendor");
                    vendor.products.insert(id, name.to_owned());
                }
            } else if let Some((id, name)) = entry(line) {
                let products = HashMap::new();
                vendors.insert(
                    id,
                    Vendor {
                        name: name.to_owned(),
                        products,
                    },
                );
                current = Some(id);
            } else {
                current = None;
            }
        }
        Self { vendors }
    }

    pub fn vendor(&self, vendor: u16) -> Option<&str> {
        Some(&self.vendors.get(&vendor)?.name)
    }

    pub fn product(&self, vendor: u16, product: u16) -> Option<&str> {
        Some(self.vendors.get(&vendor)?.products.get(&product)?)
    }
}

/// `05e3  Genesys Logic, Inc.` gives `(0x05e3, "Genesys Logic, Inc.")`.
/// Interface lines (two tabs) and class lines (`C 00`) don't match.
fn entry(line: &str) -> Option<(u16, &str)> {
    let (id, name) = line.split_once("  ")?;
    if id.len() != 4 {
        return None;
    }
    Some((u16::from_str_radix(id, 16).ok()?, name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# comment
05e3  Genesys Logic, Inc.
\t0610  Hub
\t\t00  an interface line
0781  SanDisk Corp.
\t5581  Ultra

C 00  (Defined at Interface level)
C 01  Audio
\t01  Control Device
";

    #[test]
    fn looks_up_vendors_and_products() {
        let ids = UsbIds::parse(SAMPLE);
        assert_eq!(ids.vendor(0x05e3), Some("Genesys Logic, Inc."));
        assert_eq!(ids.product(0x05e3, 0x0610), Some("Hub"));
        assert_eq!(ids.product(0x0781, 0x5581), Some("Ultra"));
        assert_eq!(ids.product(0x0781, 0x0001), None);
        assert_eq!(ids.vendor(0x1234), None);
    }

    #[test]
    fn class_section_does_not_add_products() {
        let ids = UsbIds::parse(SAMPLE);
        assert_eq!(ids.product(0x0781, 0x0001), None);
        assert_eq!(ids.vendors[&0x0781].products.len(), 1);
    }
}
