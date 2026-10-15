//! The built-in equalizer: a filter graph PipeWire runs inside an output's
//! own audio converter (`audioconvert.filter-graph.0`), plus a WirePlumber
//! rule that sets it again when the output is created.

mod live;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use gtk::glib;

pub use live::Live;

/// The graphic EQ's band centres, one octave apart.
pub const GRAPHIC_FREQS: [f64; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];
/// About one octave wide, so neighbouring bands meet.
const GRAPHIC_Q: f64 = 1.41;
/// Slider range for bands and preamp, in dB.
pub const MAX_GAIN: f64 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Peak,
    LowShelf,
    HighShelf,
}

impl Kind {
    /// The tag AutoEQ uses, which is also how Openhub stores it.
    fn tag(self) -> &'static str {
        match self {
            Kind::Peak => "PK",
            Kind::LowShelf => "LSC",
            Kind::HighShelf => "HSC",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "PK" | "PEQ" => Some(Kind::Peak),
            "LSC" | "LS" => Some(Kind::LowShelf),
            "HSC" | "HS" => Some(Kind::HighShelf),
            _ => None,
        }
    }

    fn biquad(self) -> &'static str {
        match self {
            Kind::Peak => "bq_peaking",
            Kind::LowShelf => "bq_lowshelf",
            Kind::HighShelf => "bq_highshelf",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Filter {
    pub kind: Kind,
    pub freq: f64,
    pub gain: f64,
    pub q: f64,
}

/// One output's EQ.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// dB, applied before the filters to leave headroom.
    pub preamp: f64,
    pub filters: Vec<Filter>,
    /// The imported AutoEQ file's name; empty for the graphic EQ.
    pub imported: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            preamp: 0.0,
            filters: graphic(&[0.0; 10]),
            imported: String::new(),
        }
    }
}

impl Settings {
    /// The graphic EQ's band gains, or `None` after an AutoEQ import.
    pub fn graphic_gains(&self) -> Option<[f64; 10]> {
        if !self.imported.is_empty() || self.filters.len() != GRAPHIC_FREQS.len() {
            return None;
        }
        let mut gains = [0.0; 10];
        for (gain, filter) in gains.iter_mut().zip(&self.filters) {
            *gain = filter.gain;
        }
        Some(gains)
    }
}

/// The 10 graphic bands with these gains.
pub fn graphic(gains: &[f64; 10]) -> Vec<Filter> {
    GRAPHIC_FREQS
        .iter()
        .zip(gains)
        .map(|(&freq, &gain)| Filter {
            kind: Kind::Peak,
            freq,
            gain,
            q: GRAPHIC_Q,
        })
        .collect()
}

/// The filters' combined gain at `freq`, in dB, without the preamp. It's
/// what the bars show for an imported EQ.
// ponytail: RBJ cookbook biquads at 48 kHz, the same family PipeWire uses;
// the shelves' Q may differ slightly from PipeWire's, which only moves the bars.
pub fn response(filters: &[Filter], freq: f64) -> f64 {
    const RATE: f64 = 48000.0;
    let w = std::f64::consts::TAU * freq / RATE;
    filters
        .iter()
        .map(|filter| {
            let a = 10f64.powf(filter.gain / 40.0);
            let w0 = std::f64::consts::TAU * filter.freq / RATE;
            let (sin, cos) = w0.sin_cos();
            let alpha = sin / (2.0 * filter.q);
            let root = 2.0 * a.sqrt() * alpha;
            let ([b0, b1, b2], [a0, a1, a2]) = match filter.kind {
                Kind::Peak => (
                    [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
                    [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
                ),
                Kind::LowShelf => (
                    [
                        a * ((a + 1.0) - (a - 1.0) * cos + root),
                        2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                        a * ((a + 1.0) - (a - 1.0) * cos - root),
                    ],
                    [
                        (a + 1.0) + (a - 1.0) * cos + root,
                        -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                        (a + 1.0) + (a - 1.0) * cos - root,
                    ],
                ),
                Kind::HighShelf => (
                    [
                        a * ((a + 1.0) + (a - 1.0) * cos + root),
                        -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                        a * ((a + 1.0) + (a - 1.0) * cos - root),
                    ],
                    [
                        (a + 1.0) - (a - 1.0) * cos + root,
                        2.0 * ((a - 1.0) - (a + 1.0) * cos),
                        (a + 1.0) - (a - 1.0) * cos - root,
                    ],
                ),
            };
            // |b0 + b1 e^-jw + b2 e^-2jw|² over the same for a.
            let power = |c0: f64, c1: f64, c2: f64| {
                let re = c0 + c1 * w.cos() + c2 * (2.0 * w).cos();
                let im = c1 * w.sin() + c2 * (2.0 * w).sin();
                re * re + im * im
            };
            10.0 * (power(b0, b1, b2) / power(a0, a1, a2)).log10()
        })
        .sum()
}

pub const PRESETS: [(&str, [f64; 10]); 3] = [
    ("Flat", [0.0; 10]),
    (
        "Bass Boost",
        [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    ),
    (
        "Vocal",
        [-2.0, -2.0, -1.0, 0.0, 2.0, 3.0, 3.0, 2.0, 0.0, -1.0],
    ),
];

/// The value of `audioconvert.filter-graph.0`: one `param_eq` node, with the
/// preamp as a high shelf at 0 Hz, which is a plain gain.
pub fn graph(settings: &Settings) -> String {
    let mut filters = format!(
        "{{ type = bq_highshelf freq = 0 gain = {:.2} q = 1.0 }}",
        settings.preamp
    );
    for filter in &settings.filters {
        let _ = write!(
            filters,
            " {{ type = {} freq = {:.2} gain = {:.2} q = {:.3} }}",
            filter.kind.biquad(),
            filter.freq,
            filter.gain,
            filter.q
        );
    }
    format!(
        "{{ nodes = [ {{ type = builtin name = openhub_eq label = param_eq config = {{ filters = [ {filters} ] }} }} ] }}"
    )
}

/// Reads an AutoEQ `ParametricEQ.txt`: a preamp line and peak or shelf filters.
pub fn parse_autoeq(text: &str) -> Result<(f64, Vec<Filter>), String> {
    let mut preamp = 0.0;
    let mut filters = Vec::new();
    for line in text.lines().map(str::trim) {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            ["Preamp:", value, "dB", ..] => {
                preamp = value.parse().map_err(|_| format!("Bad preamp: “{line}”"))?;
            }
            [filter, ..] if filter.starts_with("Filter") => {
                // Filter 1: ON PK Fc 105 Hz Gain -2.1 dB Q 0.70
                if words.get(2) != Some(&"ON") {
                    continue;
                }
                let kind = words.get(3).and_then(|tag| Kind::from_tag(tag));
                let number = |key: &str| {
                    let at = words.iter().position(|w| *w == key)?;
                    words.get(at + 1)?.parse::<f64>().ok()
                };
                match (kind, number("Fc"), number("Gain"), number("Q")) {
                    (Some(kind), Some(freq), Some(gain), Some(q)) if freq > 0.0 && q > 0.0 => {
                        filters.push(Filter {
                            kind,
                            freq,
                            gain,
                            q,
                        })
                    }
                    _ => return Err(format!("Unsupported filter: “{line}”")),
                }
            }
            _ => {}
        }
    }
    if filters.is_empty() {
        return Err("No filters found. Choose an AutoEQ ParametricEQ.txt file.".into());
    }
    Ok((preamp, filters))
}

/// The WirePlumber rule that sets each enabled output's graph when it is
/// created. Each match goes in both the ALSA and Bluetooth rules; it only
/// applies where that output exists.
pub fn rules(enabled: &[(String, Settings)]) -> String {
    let mut entries = String::new();
    for (name, settings) in enabled {
        let _ = write!(
            entries,
            "  {{\n    matches = [ {{ node.name = \"{name}\" }} ]\n    actions = {{ update-props = {{ audioconvert.filter-graph.0 = \"{}\" }} }}\n  }}\n",
            graph(settings)
        );
    }
    format!(
        "# Written by Openhub's equalizer. Changes here are overwritten.\nmonitor.alsa.rules = [\n{entries}]\nmonitor.bluez.rules = [\n{entries}]\n"
    )
}

fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// `wireplumber.conf.d/openhub-equalizer.conf` in the host's config folder.
fn rules_path() -> PathBuf {
    // ponytail: in Flatpak this assumes the host keeps the default
    // XDG_CONFIG_HOME (~/.config), which is what xdg-config maps to.
    let config = if in_flatpak() {
        glib::home_dir().join(".config")
    } else {
        glib::user_config_dir()
    };
    config.join("wireplumber/wireplumber.conf.d/openhub-equalizer.conf")
}

fn store_path() -> PathBuf {
    glib::user_config_dir().join("openhub/equalizer.ini")
}

/// Every output's saved EQ, by PipeWire node name.
pub fn load() -> Vec<(String, Settings)> {
    let file = glib::KeyFile::new();
    if file
        .load_from_file(store_path(), glib::KeyFileFlags::NONE)
        .is_err()
    {
        return Vec::new();
    }
    file.groups()
        .iter()
        .map(|name| {
            let name = name.as_str();
            // "PK 1000 2 1.41; LSC 105 6.7 0.7"
            let filters = file
                .string(name, "filters")
                .unwrap_or_default()
                .split(';')
                .filter_map(|filter| {
                    let parts: Vec<&str> = filter.split_whitespace().collect();
                    let [tag, freq, gain, q] = parts.as_slice() else {
                        return None;
                    };
                    Some(Filter {
                        kind: Kind::from_tag(tag)?,
                        freq: freq.parse().ok()?,
                        gain: gain.parse().ok()?,
                        q: q.parse().ok()?,
                    })
                })
                .collect::<Vec<_>>();
            let settings = Settings {
                enabled: file.boolean(name, "enabled").unwrap_or_default(),
                preamp: file.double(name, "preamp").unwrap_or_default(),
                filters: if filters.is_empty() {
                    Settings::default().filters
                } else {
                    filters
                },
                imported: file
                    .string(name, "imported")
                    .map(String::from)
                    .unwrap_or_default(),
            };
            (name.to_owned(), settings)
        })
        .collect()
}

/// Saves every output's EQ and rewrites the WirePlumber rule to match.
pub fn save(all: &[(String, Settings)]) -> Result<(), String> {
    let file = glib::KeyFile::new();
    for (name, settings) in all {
        file.set_boolean(name, "enabled", settings.enabled);
        file.set_double(name, "preamp", settings.preamp);
        let filters: Vec<String> = settings
            .filters
            .iter()
            .map(|f| format!("{} {} {} {}", f.kind.tag(), f.freq, f.gain, f.q))
            .collect();
        file.set_string(name, "filters", &filters.join("; "));
        file.set_string(name, "imported", &settings.imported);
    }
    write(&store_path(), &file.to_data())?;

    let enabled: Vec<(String, Settings)> = all
        .iter()
        .filter(|(_, settings)| settings.enabled)
        .cloned()
        .collect();
    let path = rules_path();
    if enabled.is_empty() {
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Could not remove {}: {error}", path.display()))
            }
            _ => Ok(()),
        }
    } else {
        write(&path, &rules(&enabled))
    }
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    let error = |error: glib::Error| format!("Could not save {}: {error}", path.display());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    }
    // Written to a temporary file and renamed, so a crash never leaves half a rule.
    glib::file_set_contents(path, contents.as_bytes()).map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_has_preamp_then_filters() {
        let settings = Settings {
            enabled: true,
            preamp: -6.0,
            filters: vec![Filter {
                kind: Kind::Peak,
                freq: 1000.0,
                gain: 12.0,
                q: 1.41,
            }],
            imported: String::new(),
        };
        assert_eq!(
            graph(&settings),
            "{ nodes = [ { type = builtin name = openhub_eq label = param_eq config = { filters = [ \
             { type = bq_highshelf freq = 0 gain = -6.00 q = 1.0 } \
             { type = bq_peaking freq = 1000.00 gain = 12.00 q = 1.410 } ] } } ] }"
        );
    }

    #[test]
    fn autoeq_file_parses() {
        let text = "Preamp: -6.8 dB\n\
                    Filter 1: ON LSC Fc 105 Hz Gain 6.7 dB Q 0.70\n\
                    Filter 2: ON PK Fc 2100 Hz Gain -2.6 dB Q 2.700\n\
                    Filter 3: OFF PK Fc 3000 Hz Gain 1.0 dB Q 1.0\n\
                    Filter 4: ON HSC Fc 10000 Hz Gain -3.0 dB Q 0.70\n";
        let (preamp, filters) = parse_autoeq(text).unwrap();
        assert_eq!(preamp, -6.8);
        assert_eq!(filters.len(), 3);
        assert_eq!(filters[0].kind, Kind::LowShelf);
        assert_eq!(filters[1].freq, 2100.0);
        assert_eq!(filters[2].gain, -3.0);
        assert!(parse_autoeq("hello").is_err());
        assert!(parse_autoeq("Filter 1: ON BP Fc 100 Hz Gain 1 dB Q 1").is_err());
    }

    #[test]
    fn graphic_gains_round_trip() {
        let gains = PRESETS[1].1;
        let settings = Settings {
            filters: graphic(&gains),
            ..Settings::default()
        };
        assert_eq!(settings.graphic_gains(), Some(gains));
        let imported = Settings {
            imported: "IEM.txt".into(),
            ..settings
        };
        assert_eq!(imported.graphic_gains(), None);
    }

    #[test]
    fn response_matches_filter_gains() {
        let peak = |freq, gain| Filter {
            kind: Kind::Peak,
            freq,
            gain,
            q: 1.41,
        };
        assert!((response(&[peak(1000.0, 12.0)], 1000.0) - 12.0).abs() < 0.01);
        assert!(response(&[peak(1000.0, 12.0)], 16000.0).abs() < 0.5);
        // Two filters add up.
        let both = [peak(1000.0, 6.0), peak(1000.0, -2.0)];
        assert!((response(&both, 1000.0) - 4.0).abs() < 0.01);
        let shelf = |kind| Filter {
            kind,
            freq: 1000.0,
            gain: 6.0,
            q: 0.7,
        };
        assert!((response(&[shelf(Kind::LowShelf)], 20.0) - 6.0).abs() < 0.1);
        assert!((response(&[shelf(Kind::HighShelf)], 20000.0) - 6.0).abs() < 0.3);
        assert!(response(&[shelf(Kind::HighShelf)], 20.0).abs() < 0.1);
    }

    #[test]
    fn rules_match_by_node_name() {
        let rules = rules(&[("alsa_output.usb-DAC".into(), Settings::default())]);
        assert!(rules.contains("monitor.alsa.rules = ["));
        assert!(rules.contains("monitor.bluez.rules = ["));
        assert_eq!(
            rules.matches("node.name = \"alsa_output.usb-DAC\"").count(),
            2
        );
    }
}
