//! The parts of a monitor's EDID the Details dialog shows, from the 128-byte
//! base block (VESA E-EDID 1.4). Extension blocks aren't needed for these.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edid {
    /// The three-letter PNP manufacturer ID, such as `AUS`.
    pub vendor: String,
    pub product: u16,
    /// The monitor's name descriptor (0xFC), such as "ASUS VP249".
    pub name: Option<String>,
    /// The text serial descriptor (0xFF), else the numeric serial, if set.
    pub serial: Option<String>,
    /// Laptop panels often have no name, only unspecified text (0xFE),
    /// such as the panel's part number.
    pub text: Vec<String>,
    pub year: u16,
    /// 0 when the maker gave only the year.
    pub week: u8,
    /// Physical size in centimetres; 0 for projectors.
    pub width_cm: u8,
    pub height_cm: u8,
}

impl Edid {
    /// The diagonal in inches, such as 24.0, when the size is known.
    pub fn diagonal_inches(&self) -> Option<f64> {
        let (w, h) = (f64::from(self.width_cm), f64::from(self.height_cm));
        (w > 0.0 && h > 0.0).then(|| (w * w + h * h).sqrt() / 2.54)
    }
}

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

pub fn parse(bytes: &[u8]) -> Option<Edid> {
    let base = bytes.get(..128)?;
    if base[..8] != HEADER || base.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0 {
        return None;
    }
    // Three 5-bit letters, 1 = A.
    let id = u16::from_be_bytes([base[8], base[9]]);
    let vendor: String = [10, 5, 0]
        .iter()
        .map(|shift| char::from(b'@' + ((id >> shift) & 0x1F) as u8))
        .collect();
    let mut edid = Edid {
        vendor,
        product: u16::from_le_bytes([base[10], base[11]]),
        year: 1990 + u16::from(base[17]),
        week: if base[16] <= 54 { base[16] } else { 0 },
        width_cm: base[21],
        height_cm: base[22],
        ..Edid::default()
    };
    let numeric = u32::from_le_bytes([base[12], base[13], base[14], base[15]]);
    for descriptor in base[54..126].chunks_exact(18) {
        // Display descriptors start with three zero bytes, then the tag.
        if descriptor[..3] != [0, 0, 0] {
            continue;
        }
        let text = descriptor_text(&descriptor[5..18]);
        match descriptor[3] {
            0xFC => edid.name = Some(text),
            0xFF => edid.serial = Some(text),
            0xFE => edid.text.push(text),
            _ => {}
        }
    }
    edid.serial = edid
        .serial
        .filter(|serial| !serial.is_empty())
        .or_else(|| (numeric != 0).then(|| numeric.to_string()));
    Some(edid)
}

/// A descriptor's 13 bytes of text, ending at a newline and padded with spaces.
fn descriptor_text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|b| **b != b'\n')
        .map(|b| {
            if b.is_ascii_graphic() || *b == b' ' {
                char::from(*b)
            } else {
                '?'
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// The maker's name for common PNP IDs, else the ID itself.
pub fn vendor_name(id: &str) -> &str {
    match id {
        "ACR" => "Acer",
        "AOC" => "AOC",
        "AUO" => "AU Optronics",
        "AUS" => "ASUS",
        "BNQ" => "BenQ",
        "BOE" => "BOE",
        "CMN" => "Chimei Innolux",
        "DEL" => "Dell",
        "GSM" => "LG",
        "HWP" => "HP",
        "IVM" => "Iiyama",
        "LEN" => "Lenovo",
        "LGD" => "LG Display",
        "MSI" => "MSI",
        "NEC" => "NEC",
        "PHL" => "Philips",
        "SAM" => "Samsung",
        "SDC" => "Samsung Display",
        "SHP" => "Sharp",
        "SNY" => "Sony",
        "VSC" => "ViewSonic",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/tests/fixtures/edid/{name}.bin",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(path).unwrap()
    }

    #[test]
    fn parses_an_external_monitor() {
        // An ASUS VP249, serial zeroed in the fixture.
        let edid = parse(&fixture("asus-vp249")).unwrap();
        assert_eq!((edid.vendor.as_str(), edid.product), ("AUS", 0x24AF));
        assert_eq!(edid.name.as_deref(), Some("ASUS VP249"));
        assert_eq!(edid.serial, None);
        assert_eq!((edid.year, edid.week), (2020, 30));
        assert_eq!((edid.width_cm, edid.height_cm), (53, 30));
        assert_eq!(edid.diagonal_inches().map(f64::round), Some(24.0));
        assert_eq!(vendor_name(&edid.vendor), "ASUS");
    }

    #[test]
    fn parses_a_laptop_panel() {
        // A BOE panel names itself only in unspecified-text descriptors.
        let edid = parse(&fixture("boe-nv140wum")).unwrap();
        assert_eq!(edid.vendor, "BOE");
        assert_eq!(edid.name, None);
        assert_eq!(edid.text, ["BOE CQ", "NV140WUM-N43"]);
        assert_eq!((edid.year, edid.width_cm, edid.height_cm), (2021, 30, 19));
    }

    #[test]
    fn rejects_damaged_data() {
        let mut edid = fixture("asus-vp249");
        assert!(parse(&edid[..100]).is_none());
        edid[20] ^= 0xFF; // Breaks the checksum.
        assert!(parse(&edid).is_none());
        assert!(parse(&[0; 128]).is_none());
    }
}
