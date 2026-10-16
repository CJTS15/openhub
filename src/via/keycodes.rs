//! QMK keycodes as VIA protocol 12 sends them (QMK 0.20 and later): names
//! for the keys on the layout, and the keys the picker offers.

/// Keys the picker offers, by group: (code, label).
pub fn groups() -> Vec<(&'static str, Vec<(u16, String)>)> {
    let named = |codes: std::ops::RangeInclusive<u16>| {
        codes
            .filter_map(|code| basic(code).map(|label| (code, label.to_owned())))
            .collect::<Vec<_>>()
    };
    let mut layers = Vec::new();
    for layer in 1..4u16 {
        layers.push((0x5220 | layer, format!("Hold for Layer {layer}")));
        layers.push((0x5260 | layer, format!("Toggle Layer {layer}")));
    }
    vec![
        ("Letters and Numbers", named(0x04..=0x27)),
        ("Symbols", named(0x2D..=0x38)),
        (
            "Editing",
            named(0x28..=0x2C)
                .into_iter()
                .chain([(0x00, "None".to_owned())])
                .collect(),
        ),
        (
            "Function Keys",
            named(0x3A..=0x45)
                .into_iter()
                .chain(named(0x68..=0x73))
                .collect(),
        ),
        ("Navigation", named(0x46..=0x52)),
        (
            "Modifiers",
            named(0x39..=0x39)
                .into_iter()
                .chain(named(0xE0..=0xE7))
                .collect(),
        ),
        ("Media", named(0xA5..=0xBE)),
        ("Layers", layers),
    ]
}

/// A short label for a key on the layout.
pub fn name(code: u16, custom: &[String]) -> String {
    if let Some(label) = basic(code) {
        return label.to_owned();
    }
    let layer = code & 0x1F;
    match code {
        0x0001 => "▽".to_owned(), // Transparent: the layer below decides.
        0x5200..=0x521F => format!("To {layer}"),
        0x5220..=0x523F => format!("Hold {layer}"),
        0x5240..=0x525F => format!("Base {layer}"),
        0x5260..=0x527F => format!("Toggle {layer}"),
        0x5280..=0x529F => format!("One-Shot {layer}"),
        0x52C0..=0x52DF => format!("Tap {layer}"),
        0x7800..=0x78FF => lighting(code).unwrap_or("Light").to_owned(),
        0x7E00..=0x7E1F => custom
            .get(usize::from(code - 0x7E00))
            .cloned()
            .unwrap_or_else(|| format!("Custom {}", code - 0x7E00)),
        // Modifier plus key, such as Ctrl+C.
        0x0100..=0x1FFF => {
            let mods = (code >> 8) & 0x1F;
            let names = ["Ctrl", "Shift", "Alt", "Super"];
            let mut label: Vec<&str> = names
                .iter()
                .enumerate()
                .filter(|(bit, _)| mods & (1 << bit) != 0)
                .map(|(_, name)| *name)
                .collect();
            let key = basic(code & 0xFF).unwrap_or("?");
            label.push(key);
            label.join("+")
        }
        _ => format!("{code:#06x}"),
    }
}

fn lighting(code: u16) -> Option<&'static str> {
    Some(match code {
        0x7820 => "Light On/Off",
        0x7821 => "Light Mode +",
        0x7822 => "Light Mode −",
        0x7823 => "Hue +",
        0x7824 => "Hue −",
        0x7825 => "Saturation +",
        0x7826 => "Saturation −",
        0x7827 => "Brightness +",
        0x7828 => "Brightness −",
        0x7829 => "Speed +",
        0x782A => "Speed −",
        _ => return None,
    })
}

/// The basic keys (HID usages), by their code.
fn basic(code: u16) -> Option<&'static str> {
    const LETTERS: [&str; 26] = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R",
        "S", "T", "U", "V", "W", "X", "Y", "Z",
    ];
    const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    const F_KEYS: [&str; 12] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    ];
    const F_HIGH: [&str; 12] = [
        "F13", "F14", "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    let index = |start: u16| usize::from(code - start);
    Some(match code {
        0x00 => "",
        0x04..=0x1D => LETTERS[index(0x04)],
        0x1E..=0x27 => DIGITS[index(0x1E)],
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Backspace",
        0x2B => "Tab",
        0x2C => "Space",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x32 => "#",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps Lock",
        0x3A..=0x45 => F_KEYS[index(0x3A)],
        0x46 => "Print Screen",
        0x47 => "Scroll Lock",
        0x48 => "Pause",
        0x49 => "Insert",
        0x4A => "Home",
        0x4B => "Page Up",
        0x4C => "Delete",
        0x4D => "End",
        0x4E => "Page Down",
        0x4F => "→",
        0x50 => "←",
        0x51 => "↓",
        0x52 => "↑",
        0x65 => "Menu",
        0x68..=0x73 => F_HIGH[index(0x68)],
        0xA5 => "Power",
        0xA6 => "Sleep",
        0xA7 => "Wake",
        0xA8 => "Mute",
        0xA9 => "Volume +",
        0xAA => "Volume −",
        0xAB => "Next Track",
        0xAC => "Previous Track",
        0xAD => "Stop",
        0xAE => "Play/Pause",
        0xAF => "Media Select",
        0xB0 => "Eject",
        0xB1 => "Mail",
        0xB2 => "Calculator",
        0xB3 => "My Computer",
        0xB4 => "Search",
        0xB5 => "Home Page",
        0xB6 => "Back",
        0xB7 => "Forward",
        0xB8 => "Stop Page",
        0xB9 => "Refresh",
        0xBA => "Favourites",
        0xBB => "Fast Forward",
        0xBC => "Rewind",
        0xBD => "Brightness +",
        0xBE => "Brightness −",
        0xE0 => "Left Ctrl",
        0xE1 => "Left Shift",
        0xE2 => "Left Alt",
        0xE3 => "Left Super",
        0xE4 => "Right Ctrl",
        0xE5 => "Right Shift",
        0xE6 => "Right Alt",
        0xE7 => "Right Super",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_al68s_keys() {
        // Read from the AL68's layer 0: Esc, 1, Tab, Caps Lock.
        let names: Vec<String> = [0x29, 0x1E, 0x2B, 0x39].map(|c| name(c, &[])).into();
        assert_eq!(names, ["Esc", "1", "Tab", "Caps Lock"]);
        assert_eq!(name(0x5221, &[]), "Hold 1");
        assert_eq!(name(0x7E00, &["USB".into()]), "USB");
        assert_eq!(name(0x0106, &[]), "Ctrl+C");
        assert_eq!(name(0x7FFF, &[]), "0x7fff");
    }

    #[test]
    fn picker_offers_named_keys() {
        let groups = groups();
        let letters = &groups[0].1;
        assert_eq!(letters.first(), Some(&(0x04, "A".to_owned())));
        assert!(groups.iter().all(|(_, keys)| !keys.is_empty()));
    }
}
