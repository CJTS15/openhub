//! Mice other than Logitech and Razer, through ratbagd (libratbag's daemon,
//! which Piper uses) on the system bus: DPI steps, report rate and buttons
//! of the active profile. Changes are written to the mouse by `Commit`.

mod proxies;

use zbus::zvariant::{OwnedObjectPath, Value};

use proxies::{ButtonProxy, DeviceProxy, ManagerProxy, ProfileProxy, ResolutionProxy};

/// The ratbagd API this client speaks; it has no compatibility guarantees.
const API_VERSION: i32 = 2;

/// Mapping action types.
pub const ACTION_NONE: u32 = 0;
pub const ACTION_BUTTON: u32 = 1;
pub const ACTION_SPECIAL: u32 = 2;

const SPECIAL: u32 = 1 << 30;
/// The special functions the button picker offers, by offset from 1 << 30.
pub const SPECIALS: [(u32, &str); 13] = [
    (1, "Double-Click"),
    (2, "Scroll Left"),
    (3, "Scroll Right"),
    (4, "Scroll Up"),
    (5, "Scroll Down"),
    (6, "Wheel Mode Switch"),
    (7, "DPI Cycle Up"),
    (8, "DPI Cycle Down"),
    (9, "DPI Up"),
    (10, "DPI Down"),
    (11, "DPI Shift"),
    (13, "Profile Cycle Up"),
    (14, "Profile Cycle Down"),
];

/// What a mouse button does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    None,
    Button(u32),
    Special(u32),
    /// A key or macro: shown, but changed in Piper.
    Other,
}

impl Action {
    /// From a `(uv)` mapping.
    fn from_value(mapping: &Value<'_>) -> Self {
        let Value::Structure(mapping) = mapping else {
            return Action::Other;
        };
        let (kind, value) = match mapping.fields() {
            [Value::U32(kind), value] => (*kind, value),
            _ => return Action::Other,
        };
        // The second field is itself a variant.
        let value = match value {
            Value::Value(inner) => inner,
            value => value,
        };
        let number = u32::try_from(value).ok();
        match (kind, number) {
            (ACTION_NONE, _) => Action::None,
            (ACTION_BUTTON, Some(n)) => Action::Button(n),
            (ACTION_SPECIAL, Some(n)) => Action::Special(n),
            _ => Action::Other,
        }
    }

    fn to_mapping(self) -> Option<(u32, Value<'static>)> {
        Some(match self {
            Action::None => (ACTION_NONE, Value::U32(0)),
            Action::Button(n) => (ACTION_BUTTON, Value::U32(n)),
            Action::Special(n) => (ACTION_SPECIAL, Value::U32(n)),
            Action::Other => return None,
        })
    }

    pub fn label(self) -> String {
        match self {
            Action::None => "Disabled".to_owned(),
            Action::Button(n) => button_name(n),
            Action::Special(n) => SPECIALS
                .iter()
                .find(|(offset, _)| SPECIAL + offset == n)
                .map_or_else(|| "Special".to_owned(), |(_, name)| (*name).to_owned()),
            Action::Other => "Key or Macro".to_owned(),
        }
    }

    pub fn special(offset: u32) -> Self {
        Action::Special(SPECIAL + offset)
    }
}

pub fn button_name(n: u32) -> String {
    match n {
        1 => "Left Click".to_owned(),
        2 => "Right Click".to_owned(),
        3 => "Middle Click".to_owned(),
        4 => "Back".to_owned(),
        5 => "Forward".to_owned(),
        n => format!("Button {n}"),
    }
}

/// A mouse ratbagd drives, as the dialog shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Mouse {
    pub device: OwnedObjectPath,
    pub profile: OwnedObjectPath,
    pub name: String,
    pub report_rate: Option<(u32, Vec<u32>)>,
    pub resolutions: Vec<Resolution>,
    pub buttons: Vec<Button>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    pub path: OwnedObjectPath,
    pub dpi: u32,
    /// The value is (x, y) on this mouse; written back the same way.
    pub separate_axes: bool,
    pub allowed: Vec<u32>,
    pub active: bool,
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub path: OwnedObjectPath,
    pub index: u32,
    pub action: Action,
    /// Which kinds of action this button accepts.
    pub types: Vec<u32>,
}

/// What the dialog changes; `Commit` writes it to the mouse in one go.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Changes {
    pub report_rate: Option<u32>,
    pub resolutions: Vec<(OwnedObjectPath, u32, bool)>,
    pub buttons: Vec<(OwnedObjectPath, Action)>,
}

/// Parses `usb:1038:1836:0` into the vendor and product IDs.
fn usb_ids(model: &str) -> Option<(u16, u16)> {
    let mut parts = model.split(':');
    if parts.next()? != "usb" {
        return None;
    }
    let vendor = u16::from_str_radix(parts.next()?, 16).ok()?;
    let product = u16::from_str_radix(parts.next()?, 16).ok()?;
    Some((vendor, product))
}

/// The ratbagd mouse with these USB IDs, read from its active profile.
/// `Ok(None)` when ratbagd runs but doesn't know the mouse.
pub async fn find(
    conn: &zbus::Connection,
    vendor: u16,
    product: u16,
) -> zbus::Result<Option<Mouse>> {
    let manager = ManagerProxy::new(conn).await?;
    if manager.api_version().await? != API_VERSION {
        return Ok(None);
    }
    for path in manager.devices().await? {
        let device = DeviceProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        if usb_ids(&device.model().await?) != Some((vendor, product)) {
            continue;
        }
        let mut active = None;
        for profile in device.profiles().await? {
            let proxy = ProfileProxy::builder(conn)
                .path(profile.clone())?
                .build()
                .await?;
            if proxy.is_active().await? {
                active = Some((profile, proxy));
                break;
            }
        }
        let Some((profile_path, profile)) = active else {
            return Ok(None);
        };
        let rates = profile.report_rates().await.unwrap_or_default();
        let report_rate = match profile.report_rate().await {
            Ok(rate) if !rates.is_empty() => Some((rate, rates)),
            _ => None,
        };
        let mut resolutions = Vec::new();
        for path in profile.resolutions().await? {
            let proxy = ResolutionProxy::builder(conn)
                .path(path.clone())?
                .build()
                .await?;
            if proxy.is_disabled().await.unwrap_or(false) {
                continue;
            }
            let value = proxy.resolution().await?;
            let (dpi, separate_axes) = match &*value {
                Value::U32(dpi) => (*dpi, false),
                Value::Structure(axes) => match axes.fields() {
                    [Value::U32(x), ..] => (*x, true),
                    _ => continue,
                },
                _ => continue,
            };
            resolutions.push(Resolution {
                path,
                dpi,
                separate_axes,
                allowed: proxy.resolutions().await?,
                active: proxy.is_active().await?,
                default: proxy.is_default().await?,
            });
        }
        let mut buttons = Vec::new();
        for path in profile.buttons().await? {
            let proxy = ButtonProxy::builder(conn)
                .path(path.clone())?
                .build()
                .await?;
            let mapping = proxy.mapping().await?;
            buttons.push(Button {
                path,
                index: proxy.index().await?,
                action: Action::from_value(&mapping),
                types: proxy.action_types().await?,
            });
        }
        return Ok(Some(Mouse {
            device: device.inner().path().to_owned().into(),
            profile: profile_path,
            name: device.name().await?,
            report_rate,
            resolutions,
            buttons,
        }));
    }
    Ok(None)
}

/// Sets every change, then has ratbagd write them to the mouse.
pub async fn apply(conn: &zbus::Connection, mouse: &Mouse, changes: &Changes) -> zbus::Result<()> {
    if let Some(rate) = changes.report_rate {
        let profile = ProfileProxy::builder(conn)
            .path(mouse.profile.clone())?
            .build()
            .await?;
        profile.set_report_rate(rate).await?;
    }
    for (path, dpi, separate_axes) in &changes.resolutions {
        let proxy = ResolutionProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        // The value keeps the form the mouse uses: one number, or (x, y).
        let value = if *separate_axes {
            Value::from((*dpi, *dpi))
        } else {
            Value::U32(*dpi)
        };
        proxy.set_resolution(value).await?;
    }
    for (path, action) in &changes.buttons {
        if let Some((kind, value)) = action.to_mapping() {
            let proxy = ButtonProxy::builder(conn)
                .path(path.clone())?
                .build()
                .await?;
            // A `Value` field serializes as the `v` in `(uv)`.
            proxy.set_mapping(Value::from((kind, value))).await?;
        }
    }
    DeviceProxy::builder(conn)
        .path(mouse.device.clone())?
        .build()
        .await?
        .commit()
        .await
}

/// The allowed value nearest to `dpi`, since ratbagd takes only those.
pub fn nearest(allowed: &[u32], dpi: u32) -> u32 {
    allowed
        .iter()
        .copied()
        .min_by_key(|value| value.abs_diff(dpi))
        .unwrap_or(dpi)
}

#[cfg(test)]
mod tests;
