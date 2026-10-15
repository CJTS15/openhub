# Openhub v0.3 plan: Power

Oct 14, 2026 · @CJ

## Summary

v0.3 adds a Power page that shows every battery UPower knows about in one place: the laptop's own battery and charger, plus wireless mice, keyboards, headsets, gamepads and pens, whether they connect over Bluetooth or a USB receiver. It reads everything from UPower over D-Bus with signals, so unlike the USB page there is no polling. It also feeds peripheral battery levels back onto the USB page, which v0.2 deferred. Nothing is coded yet; this plan comes first, and v0.3 starts after v0.2 is tagged (M8).

Defaults where the scope forks (CJ confirmed or changed them on 2026-10-14, see Decisions):

- **Data source:** UPower's D-Bus API only (`org.freedesktop.UPower`), no direct reads of `/sys/class/power_supply` except to find which USB device a battery belongs to.
- **Live updates:** UPower's `DeviceAdded`, `DeviceRemoved` and `PropertiesChanged` signals. No timer.
- **One action:** the charge limit ("Preserve battery health") through UPower, hidden on laptops or UPower versions that don't support it. Power mode is shown nowhere and not switched; GNOME Settings owns it.
- **Bluetooth page:** unchanged. It keeps reading battery from BlueZ, so it still works on systems without UPower.
- **USB page:** gains a battery level on rows whose device owns a battery (for example a Logitech receiver and its mouse), taken from UPower.
- **Duplicates:** a device UPower reports twice (once through BlueZ, once through the kernel's HID driver) shows once on the Power page.

## Scope and actions

Power on Linux is mostly something to read: UPower reports it, and only a few settings can be changed without root. So v0.3 is a clear, live view of every battery plus one action that protects the laptop battery, the charge limit.

| Feature | v0.3 | How it works | Why |
| --- | --- | --- | --- |
| Overall status: on battery or plugged in, %, time left or time to full | Yes | UPower `GetDisplayDevice` and `OnBattery` | The one line people look for first |
| Laptop batteries, chargers and UPS units with live level and state | Yes | UPower devices with `PowerSupply = true` | Core of the page |
| Peripheral batteries: mice, keyboards, headsets, gamepads, pens, phones | Yes | UPower devices with `PowerSupply = false` | Covers Bluetooth and USB-receiver devices in one list |
| Battery health: capacity vs design, charge cycles, technology, energy rate, voltage | Yes | UPower `Device` properties | Answers "is my battery worn out?" |
| Low-battery warning colour on rows | Yes | UPower `WarningLevel` | In-app only; the desktop already sends notifications |
| Battery level on USB rows | Yes | UPower matched to the USB device through sysfs | Deferred from v0.2 |
| Charge limit ("Preserve battery health") | Yes | UPower `ChargeThresholdSupported`, `ChargeEndThreshold` and `EnableChargeThreshold` | Needs UPower 1.90 or newer and a laptop that exposes charge thresholds; GNOME Settings added the same switch in GNOME 48 ([LinuxConfig](https://linuxconfig.org/?p=25616)). Hidden when unsupported |
| Power mode (Power Saver, Balanced, Performance) | No | power-profiles-daemon | CJ chose to keep v0.3 read-only apart from the charge limit; GNOME Settings and Quick Settings own it |
| Charge history graph | Later | UPower `GetHistory` | Nice, but a chart widget is its own milestone |
| Low-battery notifications, suspend on lid close, screen blanking | No | GNOME Settings and the desktop's power manager | Already owned by the desktop |
| CPU governors, TLP-style tuning | No | Root tools | Out of scope for a device manager |

Out of scope for v0.3: battery data from devices UPower doesn't know about (for example game controllers that only report through a vendor tool) and any power setting that needs root.

## How it relates to battery on the Bluetooth and USB pages

The Power page is the one place that lists every battery; the Bluetooth and USB pages keep showing battery next to the device it belongs to. UPower already collects batteries from BlueZ and from the kernel's HID drivers, so the Power page gets Bluetooth and USB peripherals without talking to either stack.

| Page | Battery source | What it shows | Change in v0.3 |
| --- | --- | --- | --- |
| Bluetooth | BlueZ `Battery1` (shipped in M3) | "Connected · 80% battery" on the row, Battery in details | None. BlueZ stays the source so the page works without UPower |
| USB | UPower, matched by sysfs path | Battery on the row of the device that owns it, for example a wireless receiver showing its mouse at 80% | New in M11 |
| Power | UPower | Every battery: this computer, then peripherals, each labelled Bluetooth, USB or Other | New page |

Two details worth calling out:

- **Matching a battery to a USB row.** For kernel batteries UPower's `NativePath` is the power-supply name (for example `hidpp_battery_0`). Resolving `/sys/class/power_supply/<name>/device` gives a sysfs path, and the USB row whose path is a prefix of it owns the battery. A receiver with two devices (mouse and keyboard) lists both in its details and shows the lower level on its row. Inferred from UPower's Linux backend; M11 confirms it on CJ's hardware.
- **Bluetooth devices reported twice.** Some Bluetooth mice show up in UPower once through BlueZ (`NativePath` starts with `/org/bluez/`) and once through the kernel HID driver. The Power page merges entries with the same serial (the Bluetooth address) and prefers the BlueZ one. Also inferred; the M9 fixture tests cover the merge rule and CJ's devices confirm it.

The Bluetooth and Power pages can briefly disagree by a percent or two, because BlueZ and UPower update on their own schedules. That is acceptable and not worth syncing.

## Flatpak sandbox

v0.3 needs one new manifest line, a system-bus name, and no device or filesystem access. UPower runs on the host, so the Flatpak and a native build behave the same, and the GNOME 50 runtime version doesn't matter. The charge limit goes through the same UPower name; UPower's polkit policy decides whether it asks for a password, which M10 checks on CJ's laptop.

| Permission | What it gives | Use in v0.3 |
| --- | --- | --- |
| `--system-talk-name=org.freedesktop.UPower` | Every battery, the display device, `OnBattery`, live signals, the charge limit | **Yes, the data source and the one action** |
| `--system-talk-name=org.freedesktop.UPower.PowerProfiles` | Read and set the power mode | No, power mode is out of scope |
| Default sysfs (read-only) | `/sys/class/power_supply` links | Already there; used only to match batteries to USB rows |

## Architecture

Power is a sibling of the Bluetooth and USB stacks: a `src/upower/` service, a GObject model and a `src/ui/power/` page, with no new crates. Everything stays on the GTK main thread; D-Bus calls are async zbus futures on `glib::spawn_future_local`, exactly as in `src/bluez/`.

&#91;embedded content: Openhub v0.3 architecture · 3 stacks, new Power stack highlighted\]

Each page reads only its model and each service is the only code that talks to its system; the dashed line is the USB page reading batteries from the Power model.

Proposed layout:

- `src/upower/proxies.rs`: zbus proxies for `org.freedesktop.UPower` and `org.freedesktop.UPower.Device`
- `src/upower/convert.rs`: plain functions that turn property maps into structs: the `Type`, `State` and `BatteryLevel` enums, time formatting, the duplicate merge and the connection label. No GObjects, so it is unit-tested without a bus
- `src/upower/service.rs`: `PowerManager`, which calls `EnumerateDevices`, listens for `DeviceAdded`, `DeviceRemoved` and each device's `PropertiesChanged`, keeps the display device current, and calls `EnableChargeThreshold`
- `src/model/power_device.rs`: GObject with the properties the UI binds to, including `sysfs-path` (resolved for kernel batteries, empty otherwise), `serial`, and the charge-limit state
- `src/ui/power/`: `page.rs`, `device_row.rs`, `battery_dialog.rs`, plus `data/resources/ui/power_page.ui`

Two details worth calling out:

- **One PowerManager for the window.** It starts when the window opens, not when the Power page is shown, because the USB page needs it too. It costs nothing while idle: no timer, only signals.
- **No coupling between services.** `UsbManager` doesn't know about UPower. The USB page asks the shared power model for batteries under a device's sysfs path and binds the result, so either service can be missing without breaking the other.

## UI

The Power page is a third sidebar entry after Bluetooth and USB, built from the same libadwaita pieces and within the GTK 4.14 / libadwaita 1.5 floor.

| Part | libadwaita building blocks | What it shows |
| --- | --- | --- |
| Status header | `AdwPreferencesGroup` with a `GtkLevelBar` | "On battery · 64% · 3 h 10 min left" or "Plugged in · Charging · full in 45 min"; hidden on a desktop with no battery |
| This computer | `AdwPreferencesGroup`, `AdwActionRow` per battery, charger or UPS | Name, level, state; the charger row says Connected or Not connected. Rows open the details dialog |
| Charge limit | `AdwSwitchRow` under the laptop battery | "Preserve battery health", subtitle "Stops charging at 80%" with the laptop's real end threshold; a toast if UPower refuses. Hidden when unsupported |
| Devices | `AdwPreferencesGroup`, `AdwActionRow` per peripheral | Type icon, model name, level, "Bluetooth" / "USB" / "Other"; warning colour at low and critical levels. Devices that only report coarse levels show Full, High, Low or Critical instead of a percentage. Plain rows, no link to other pages |
| Battery details | `AdwDialog` with `AdwActionRow`s | Health (capacity vs design, %), energy now / full / design (Wh), charge cycles, energy rate (W), voltage, technology, vendor, model, serial |
| Empty and error states | `AdwStatusPage` | UPower not running, or no batteries at all |
| USB page addition | Row subtitle and a Battery row in the existing details dialog | "80% battery" on the row of the device that owns it; a receiver with two devices shows the lower level |

## Milestones

v0.3 starts once v0.2 is tagged (M8). Each milestone ends with something CJ can run on a real machine. The cloud container has no battery, so CI runs the page against a fake UPower and power-profiles-daemon from python-dbusmock, which ships templates for both.

1. **M9 Power list**: UPower proxies, `PowerManager`, the Power sidebar entry, status header, This computer and Devices groups with live updates from signals, duplicate merge, empty and error states, and the `org.freedesktop.UPower` permission in the manifest. Unit tests cover the enum mapping, time formatting and merge rule; a dbusmock test in CI adds and removes a battery.
2. **M10 Battery details and charge limit**: details dialog with health, energy, cycles, rate, voltage and identity; coarse-level devices; warning colours; the "Preserve battery health" switch, hidden when unsupported, with a dbusmock test for both cases.
3. **M11 Peripherals on the USB page**: resolve each kernel battery's sysfs path, show battery on the owning USB row and in its details, and handle a receiver with more than one device. CJ's Yunzii AL68 2.4 GHz dongle doesn't report battery, so on hardware it checks the no-battery case (no level, no error); the battery case is covered by fixtures and dbusmock only.
4. **M12 Packaging v0.3**: AppStream release notes, CJ tests the page and the charge limit on a laptop inside the Flatpak, then a v0.3.0 tag.

After v0.3, natural next steps are a charge history graph on this page, or another category such as displays or audio devices.

## Risks and open questions

Risks:

- **Peripheral coverage depends on the kernel.** UPower sees a wireless device only if its driver exposes a battery (Logitech HID++ and most Bluetooth devices do; many 2.4 GHz receivers, including CJ's Yunzii AL68 dongle, don't). Devices it can't see simply don't appear; Openhub adds no vendor-specific code.
- **Duplicate and USB matching rules are inferred**, not tested on hardware. M9 confirms the merge on CJ's Bluetooth devices; with no battery-reporting receiver at hand, the USB match rests on fixtures. If a rule is wrong, the fallback is showing both entries rather than guessing.
- **Charge limit support is patchy.** It needs UPower 1.90 or newer and a laptop whose firmware exposes charge thresholds; Ubuntu 24.04's UPower may predate it. The switch is hidden in those cases, and the exact property names are checked against the installed UPower in M10.
- **No UPower** (minimal window managers): the Power sidebar entry shows the status page and the USB page simply shows no battery. Bluetooth is unaffected.

Open questions for CJ:

- [x] Is the power mode switch the right single action for v0.3, or should v0.3 stay read-only?
- [x] Charge limit: leave it for later (my default), or add it in v0.3, hidden on systems that don't support it?
- [x] Should a Bluetooth device's row on the Power page jump to that device on the Bluetooth page, or stay a plain row (my default)?
- [x] On the USB page, should a receiver show its devices' battery on the row (my default) or only in its details?
- [x] Do you have a wireless peripheral on a USB receiver to test M11 with? If not, M11 can only be checked with fixtures.

## Decisions

CJ answered the open questions on 2026-10-14.

| Question | Decision |
| --- | --- |
| Power mode | Not in v0.3; nothing switches power profiles |
| Charge limit | In v0.3 (M10), hidden where unsupported; it is the page's one action |
| Bluetooth rows on the Power page | Plain rows, no link to the Bluetooth page |
| Receiver battery on the USB page | Shown on the row |
| Test hardware for M11 | Yunzii AL68 2.4 GHz dongle, which doesn't report battery: tests the no-battery case; the battery case uses fixtures |
