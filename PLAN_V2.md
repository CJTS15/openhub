# Openhub v0.2 plan: USB devices

Oct 13, 2026 · @CJ

## Summary

v0.2 adds a USB page that lists every USB device with live plug and unplug, shows its details, and safely ejects USB drives. It reads device data from sysfs, which the Flatpak sandbox already exposes read-only, so listing needs no new permission. Ejecting goes through UDisks2 over D-Bus, the same way GNOME Files does it. Nothing is coded yet; this plan comes first.

Defaults I picked where the scope forks (CJ confirmed them on 2026-10-13, see Decisions):

- **Data source:** read `/sys/bus/usb/devices` directly in Rust, no libudev and no new crate.
- **Live updates:** re-scan sysfs every 2 seconds, only while the USB page is on screen. The USB portal is not used in v0.2.
- **Actions:** device details, copy IDs, and safe eject for storage. No driver unbinding, port power, or authorize/deauthorize, because those need root.
- **Names:** the device's own manufacturer and product strings first, then a bundled `usb.ids` lookup, then the raw `vendor:product` ID.
- **Hubs:** shown as a tree under each root port, collapsed by default; internal root hubs are hidden.

## Scope and management actions

USB management on Linux is mostly read-only for a normal user: anything that changes the bus itself needs root. So v0.2 is a clear, live inventory plus one action that matters every day, safe eject.

| Action | v0.2 | How it works | Why |
| --- | --- | --- | --- |
| List devices with live plug and unplug | Yes | sysfs scan | Readable in the sandbox with no permission |
| Details: vendor, product, serial, IDs, speed, USB version, class, port path, driver, max power | Yes | sysfs attributes | Same |
| Copy `vendor:product` ID and serial | Yes | Clipboard | Useful for bug reports and udev rules |
| Safely eject a USB drive (unmount, then power off) | Yes | UDisks2 `Filesystem.Unmount` and `Drive.PowerOff` | Polkit already allows this for the active user, no password |
| Battery level of USB mice, keyboards, headsets | Later | UPower | Small, but a separate D-Bus service; better as v0.3 with a Power page |
| Authorize or block a device | No | USBGuard D-Bus, if installed | Rarely installed; root policy tool, not a desktop feature |
| Unbind or rebind a driver, reset a port | No | sysfs write as root | Needs root and a privileged helper |
| Autosuspend and port power control | No | sysfs write as root | Same |
| Thunderbolt and USB4 authorization | No | bolt daemon | GNOME Settings already covers it |
| Raw device access (firmware updates, vendor tools) | No | USB portal or `--device=usb` | Not a device manager's job; fwupd and GNOME Firmware cover firmware |

Out of scope for v0.2: storage beyond eject (formatting, partitions: GNOME Disks owns that), webcams and audio interfaces as their own categories, and USB network adapters.

## Flatpak sandbox

The whole v0.2 feature set needs one new manifest line, `--system-talk-name=org.freedesktop.UDisks2`, and no device access. Flatpak shares `/sys/bus`, `/sys/class`, `/sys/dev` and `/sys/devices` read-only by default, which is everything needed to list USB devices and read their descriptors ([Flatpak sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)).

| Option | What it gives | Use in v0.2 |
| --- | --- | --- |
| Default sysfs (read-only) | Every USB device, its attributes and the hub tree | **Yes, the data source** |
| `/run/udev` and the hwdb | udev's database names and tags | Not in the sandbox, so names come from the device or a bundled `usb.ids` |
| udev monitor (libudev) | Hotplug events | Not used: depends on udevd's socket and `/run/udev`, which the sandbox lacks. Inferred, to confirm in M5 |
| `--device=usb` (Flatpak 1.15.11+) | Raw `/dev/bus/usb` nodes | No: Openhub never opens a device, and Flathub reviewers push back on it |
| `--device=all` | Every device node | No, same reason and broader |
| USB portal + `--usb=` | Device list, hotplug events, and opening a device after a user prompt | No: needs xdg-desktop-portal 1.20 (USB portal first shipped in 1.19.1, [Phoronix](https://phoronix.com/news/XDG-Desktop-Porta-1.19.1)); Ubuntu 24.04 ships an older portal, and the portal exists to hand out device file descriptors, which Openhub does not need |
| `--system-talk-name=org.freedesktop.UDisks2` | Unmount and power off drives through polkit | **Yes, for eject** |

What this means in practice: the Flatpak and a native build behave the same, and running on older systems costs nothing. The one gap is hotplug, which the 2-second re-scan covers; a native-only netlink listener or the USB portal's `DeviceEvents` can replace polling later without changing the UI.

## Architecture

USB is a sibling of the Bluetooth stack: a `src/usb/` service, a GObject model, and a `src/ui/usb/` page, with no new crates. Everything stays on the GTK main thread as in v0.1; sysfs reads are a few dozen small files, so they run inline, and UDisks2 calls are async zbus futures on `glib::spawn_future_local`.

Proposed layout:

- `src/usb/sysfs.rs`: one function that walks `/sys/bus/usb/devices` and returns plain structs (no GObjects), so it can be unit-tested against a fake sysfs tree in `tests/fixtures/`
- `src/usb/ids.rs`: lazy `usb.ids` lookup from the first file found in `/app/share/hwdata`, `/usr/share/hwdata`, `/usr/share/misc`
- `src/usb/service.rs`: `UsbManager`, which owns the device list, re-scans on a `glib::timeout_add_seconds_local` timer while the page is mapped, and diffs by sysfs name (for example `1-2.3`) so rows update in place instead of flickering
- `src/usb/udisks.rs`: zbus proxies for UDisks2 `ObjectManager`, `Block`, `Filesystem` and `Drive`, plus eject
- `src/model/usb_device.rs`: GObject with the properties the UI binds to
- `src/ui/usb/`: `page.rs`, `device_row.rs`, `device_dialog.rs`, plus `data/resources/ui/usb_page.ui`

Two details worth calling out:

- **Finding the drive to eject.** A USB stick's block device sits under its USB device in sysfs (`.../2-1/2-1:1.0/host*/.../block/sdb`). The scan records `sdb`, and the service matches it to the UDisks2 block object whose `Device` is `/dev/sdb`. No guessing by vendor or serial.
- **Window change.** `window.ui` currently hard-codes the Bluetooth page as the content pane. M5 turns the sidebar into a real category switch (a `GtkStack` with one child per category) so USB, and later categories, slot in.

The diagram below shows where USB sits next to the existing Bluetooth stack.

&#91;embedded content: Openhub v0.2 architecture · 3 layers, 3 system services\]

The USB page reads only its model; the two new services are the only USB code that touches sysfs or D-Bus.

## Milestones

v0.2 starts once v0.1 is tagged (M4 packaging, PR #6). Each milestone ends with something CJ can run on a real machine; the cloud container has no USB hardware, so it proves the build and the sysfs parser through fixture tests only.

1. **M5 Category switch and USB list**: sidebar becomes a real category switch with a USB entry; sysfs scan, `usb.ids` names, hub tree with `AdwExpanderRow`, device type icons from the USB class, an empty state, and a "Show internal devices" toggle, off by default. Unit tests run the parser against fixture trees (a hub, a phone, a USB stick, a device with no strings).
2. **M6 Live updates and details**: 2-second re-scan while the page is visible, diffed so rows update in place; details dialog with IDs, serial, speed, USB version, class, driver, max power and port path; copy buttons for the ID and serial.
3. **M7 Safe eject**: UDisks2 proxies, an Eject button on storage rows, unmount then power off, "Safe to remove" and "Drive is busy" toasts, and the `org.freedesktop.UDisks2` permission in the manifest.
4. **M8 Packaging v0.2**: `usb.ids` shipped in the Flatpak, AppStream release notes, CJ tests list, hotplug and eject inside the Flatpak, then a v0.2.0 tag.

After v0.2, the natural v0.3 is a Power page (UPower batteries, which also covers USB and Bluetooth peripherals).

## Risks and open questions

Risks:

- **Hotplug in the sandbox is inferred, not tested.** If sysfs inside the Flatpak turns out to lag or hide new devices, M5 testing will show it and the fallback is the USB portal on systems that have it.
- **Eject on systems without UDisks2** (minimal window managers): the Eject button is hidden and the page still lists devices.
- **Polling cost** is small (one directory walk every 2 s, only while visible) but is the first thing to swap for events if it shows up in profiling.

Open questions for CJ:

- [x] Is safe eject the right single action for v0.2, or do you want something else in its place?
- [x] Hub tree (my default) or a flat list grouped by type (Input, Storage, Audio, Other)?
- [x] Show internal devices such as built-in webcams, fingerprint readers and Bluetooth radios, or hide them behind a "Show internal devices" toggle? I lean to the toggle, off by default.
- [x] Is a 2-second refresh while the page is open acceptable, or should hotplug feel instant (which means the USB portal and a newer-systems-only path)?

## Decisions

CJ answered the open questions on 2026-10-13.

| Question | Decision |
| --- | --- |
| Management action for v0.2 | Safe eject only |
| Layout | Hub tree under each root port, collapsed by default |
| Internal devices | Hidden behind a "Show internal devices" toggle, off by default |
| Live updates | 2-second re-scan while the USB page is visible; no USB portal |