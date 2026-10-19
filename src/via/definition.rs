//! A VIA keyboard definition: the JSON file the VIA app loads, which says
//! what the protocol can't: where each key sits, and which lighting controls
//! the keyboard has (with its own effect names).

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Definition {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub keys: Vec<Key>,
    /// Lighting and other settings, in the menu's order.
    pub controls: Vec<Control>,
    /// Names for the keyboard's own keycodes, from 0x7E00 (QK_KB_0) on.
    pub custom_keycodes: Vec<String>,
}

/// One key on the layout, in key units (1 = a letter key).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Key {
    pub row: u8,
    pub col: u8,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    /// The menu section, such as "Backlight".
    pub section: String,
    pub label: String,
    pub kind: ControlKind,
    /// The ID other controls' `showIf` refer to, such as `id_qmk_rgb_matrix_effect`.
    pub id: String,
    pub channel: u8,
    pub value_id: u8,
    pub show_if: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlKind {
    Range(u8, u8),
    /// (label, value) pairs.
    Dropdown(Vec<(String, u8)>),
    Toggle,
    /// Hue and saturation, two bytes.
    Color,
}

impl ControlKind {
    /// How many bytes the value takes.
    pub fn len(&self) -> usize {
        match self {
            ControlKind::Color => 2,
            _ => 1,
        }
    }
}

impl Definition {
    pub fn parse(text: &str) -> Result<Self, String> {
        let json: Value =
            serde_json::from_str(text).map_err(|e| format!("Not a VIA definition: {e}"))?;
        let id = |key: &str| {
            json[key]
                .as_str()
                .and_then(|text| u16::from_str_radix(text.trim_start_matches("0x"), 16).ok())
                .ok_or_else(|| format!("Not a VIA definition: no {key}"))
        };
        let rows = json["layouts"]["keymap"]
            .as_array()
            .ok_or("Not a VIA definition: no layouts.keymap")?;
        let mut controls = Vec::new();
        for menu in json["menus"].as_array().into_iter().flatten() {
            for section in menu["content"].as_array().into_iter().flatten() {
                let title = section["label"].as_str().unwrap_or_default();
                for item in section["content"].as_array().into_iter().flatten() {
                    if let Some(control) = control(title, item) {
                        controls.push(control);
                    }
                }
            }
        }
        Ok(Self {
            name: json["name"].as_str().unwrap_or("Keyboard").to_owned(),
            vendor: id("vendorId")?,
            product: id("productId")?,
            keys: layout(rows),
            controls,
            custom_keycodes: json["customKeycodes"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|code| {
                    code["shortName"]
                        .as_str()
                        .or(code["title"].as_str())
                        .or(code["name"].as_str())
                        .unwrap_or("?")
                        .to_owned()
                })
                .collect(),
        })
    }
}

/// KLE rows: strings are keys ("row,col", plus layout-option and encoder
/// marks after newlines), objects change the next key's offset and size.
fn layout(rows: &[Value]) -> Vec<Key> {
    let mut keys = Vec::new();
    let mut y = 0.0;
    for row in rows {
        let mut x = 0.0;
        let (mut w, mut h) = (1.0, 1.0);
        for item in row.as_array().into_iter().flatten() {
            match item {
                Value::Object(props) => {
                    let number = |key: &str| props.get(key).and_then(Value::as_f64);
                    // Imported files are untrusted: keep keys on a sane board.
                    x = (x + number("x").unwrap_or(0.0)).clamp(0.0, 40.0);
                    y = (y + number("y").unwrap_or(0.0)).clamp(0.0, 40.0);
                    w = number("w").unwrap_or(w).clamp(0.25, 10.0);
                    h = number("h").unwrap_or(h).clamp(0.25, 10.0);
                }
                Value::String(label) => {
                    let lines: Vec<&str> = label.split('\n').collect();
                    // "0,1" in line 4 is layout option 0, choice 1: only the
                    // default choice (0) is drawn. Line 10 "e" is an encoder.
                    let other_choice = lines
                        .get(3)
                        .and_then(|option| option.split_once(','))
                        .is_some_and(|(_, choice)| choice.trim() != "0");
                    let encoder = lines.get(9).is_some_and(|mark| mark.starts_with('e'));
                    let position = lines[0]
                        .split_once(',')
                        .and_then(|(r, c)| Some((r.trim().parse().ok()?, c.trim().parse().ok()?)));
                    if let (Some((row, col)), false, false) = (position, other_choice, encoder) {
                        keys.push(Key {
                            row,
                            col,
                            x,
                            y,
                            w,
                            h,
                        });
                    }
                    x += w;
                    (w, h) = (1.0, 1.0);
                }
                _ => {}
            }
        }
        y += 1.0;
    }
    keys
}

fn control(section: &str, item: &Value) -> Option<Control> {
    // ["id_qmk_rgb_matrix_brightness", 3, 1]: ID, channel, value ID.
    let content = item["content"].as_array()?;
    let id = content.first()?.as_str()?.to_owned();
    let channel = u8::try_from(content.get(1)?.as_u64()?).ok()?;
    let value_id = u8::try_from(content.get(2)?.as_u64()?).ok()?;
    let options = item["options"].as_array();
    let kind = match item["type"].as_str()? {
        "range" => {
            let bound = |i: usize| {
                options
                    .and_then(|o| o.get(i))
                    .and_then(Value::as_u64)
                    .and_then(|v| u8::try_from(v).ok())
            };
            ControlKind::Range(bound(0).unwrap_or(0), bound(1).unwrap_or(255))
        }
        "dropdown" => ControlKind::Dropdown(
            options?
                .iter()
                .enumerate()
                .filter_map(|(index, option)| match option {
                    // A plain label is its index; ["label", value] sets the value.
                    Value::String(label) => Some((label.clone(), u8::try_from(index).ok()?)),
                    Value::Array(pair) => Some((
                        pair.first()?.as_str()?.to_owned(),
                        u8::try_from(pair.get(1)?.as_u64()?).ok()?,
                    )),
                    _ => None,
                })
                .collect(),
        ),
        "toggle" => ControlKind::Toggle,
        "color" => ControlKind::Color,
        _ => return None, // ponytail: "keycode" menu items aren't shown yet.
    };
    Some(Control {
        section: section.to_owned(),
        label: item["label"].as_str().unwrap_or_default().to_owned(),
        kind,
        id,
        channel,
        value_id,
        show_if: item["showIf"].as_str().map(str::to_owned),
    })
}

/// Evaluates a `showIf` such as "{id_effect} != 0 && {id_effect} != 24"
/// against the controls' first value bytes. Anything it can't read shows.
pub fn shows(condition: &str, values: &HashMap<String, u8>) -> bool {
    condition.split("||").any(|any| {
        any.split("&&").all(|clause| {
            let clause = clause.trim().trim_matches(|c| c == '(' || c == ')');
            let Some(rest) = clause.strip_prefix('{') else {
                return true;
            };
            let Some((id, rest)) = rest.split_once('}') else {
                return true;
            };
            let rest = rest.trim();
            let (op, number) = ["==", "!=", "<=", ">=", "<", ">"]
                .iter()
                .find_map(|op| Some((*op, rest.strip_prefix(op)?.trim())))
                .unwrap_or(("", ""));
            let (Some(value), Ok(number)) = (values.get(id.trim()), number.parse::<u8>()) else {
                return true;
            };
            match op {
                "==" => *value == number,
                "!=" => *value != number,
                "<=" => *value <= number,
                ">=" => *value >= number,
                "<" => *value < number,
                ">" => *value > number,
                _ => true,
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small definition in the AL68's shape (not Yunzii's file itself).
    const SAMPLE: &str = r#"{
        "name": "Sample 65",
        "vendorId": "0x4D4B",
        "productId": "0x304E",
        "matrix": {"rows": 2, "cols": 3},
        "layouts": {"keymap": [
            ["0,0", {"w": 2}, "0,1", "0,2\n\n\n0,1", "0,2\n\n\n0,0"],
            [{"x": 0.5, "w": 1.5}, "1,0", "1,1\n\n\n\n\n\n\n\n\ne"]
        ]},
        "customKeycodes": [{"name": "KC_USB", "title": "USB MODE", "shortName": "USB"}],
        "menus": [{"label": "Lighting", "content": [{"label": "Backlight", "content": [
            {"label": "Brightness", "type": "range", "options": [0, 255],
             "content": ["id_brightness", 3, 1]},
            {"label": "Effect", "type": "dropdown", "options": ["Off", "Solid", ["Wave", 9]],
             "content": ["id_effect", 3, 2]},
            {"showIf": "{id_effect} != 0", "label": "Color", "type": "color",
             "content": ["id_color", 3, 4]}
        ]}]}]
    }"#;

    #[test]
    fn parses_ids_layout_and_menus() {
        let def = Definition::parse(SAMPLE).unwrap();
        assert_eq!((def.vendor, def.product), (0x4D4B, 0x304E));
        assert_eq!(def.custom_keycodes, ["USB"]);
        let at = |row, col| {
            def.keys
                .iter()
                .find(|k| (k.row, k.col) == (row, col))
                .copied()
        };
        assert_eq!(at(0, 1).map(|k| (k.x, k.w)), Some((1.0, 2.0)));
        // Layout option 0, choice 0 only: drawn once, after the skipped choice.
        assert_eq!(
            def.keys.iter().filter(|k| (k.row, k.col) == (0, 2)).count(),
            1
        );
        assert_eq!(at(0, 2).map(|k| k.x), Some(4.0));
        assert_eq!(at(1, 0).map(|k| (k.x, k.y, k.w)), Some((0.5, 1.0, 1.5)));
        assert_eq!(at(1, 1), None); // An encoder.
        assert_eq!(def.controls.len(), 3);
        assert_eq!(def.controls[0].kind, ControlKind::Range(0, 255));
        assert_eq!(
            def.controls[1].kind,
            ControlKind::Dropdown(vec![
                ("Off".into(), 0),
                ("Solid".into(), 1),
                ("Wave".into(), 9)
            ])
        );
        assert_eq!((def.controls[2].channel, def.controls[2].value_id), (3, 4));
        assert!(Definition::parse("{}").is_err());
        // Absurd sizes from a bad file stay on the board.
        let huge = r#"{"vendorId": "0x1", "productId": "0x2",
            "layouts": {"keymap": [[{"x": -5, "w": 1e9, "h": -3}, "0,0"]]}}"#;
        let key = Definition::parse(huge).unwrap().keys[0];
        assert_eq!((key.x, key.w, key.h), (0.0, 10.0, 0.25));
    }

    #[test]
    fn show_if_conditions() {
        let values = HashMap::from([("id_effect".to_owned(), 4u8)]);
        assert!(shows("{id_effect} != 0", &values));
        assert!(!shows("{id_effect} != 0 && {id_effect} != 4", &values));
        assert!(shows("{id_effect} == 1 || {id_effect} == 4", &values));
        assert!(shows("{id_unknown} == 1", &values));
    }
}
