# Openhub: Bluetooth-first plan

Oct 13, 2026 · @CJ

## Goal and scope

The first release is a native GNOME-style app that does what the Bluetooth panel in Settings does, done well: see adapters and devices, pair, connect, disconnect, forget, and power the adapter on and off. Nothing has been coded yet; this plan comes first.

In scope for v0.1:

- Adapter: power on/off, discoverable toggle, rename, start/stop scanning
- Devices: paired list and nearby list, live updates as devices appear, connect and disconnect
- Pairing: pair, trust, forget, with a PIN/passkey confirmation dialog
- Device details: name, type icon, address, battery level when the device reports it
- Graceful states: no adapter, Bluetooth service not running, adapter blocked by rfkill

Out of scope for v0.1: file transfer (OBEX), audio profile switching, multiple-window or tray mode, and other device classes (USB, displays, storage) which come after Bluetooth.

## Architecture

One Cargo crate with three layers: widgets, a GObject model they bind to, and a service module that is the only code that talks to BlueZ.

&#91;embedded content: Openhub architecture · 3 layers in the app, BlueZ outside\]

Everything runs on the GTK main thread: D-Bus calls are async and are spawned with `glib::spawn_future_local`, so the UI never blocks and no locks or channels are needed. Keeping D-Bus behind the service layer means a later USB or battery category adds a sibling module without touching the widgets.

Proposed layout:

- `src/main.rs`, `application.rs`, `window.rs`: app startup and the split-view window
- `src/bluez/`: `proxies.rs` (zbus proxy traits), `agent.rs` (pairing agent), `service.rs` (device discovery and commands)
- `src/model/`: `adapter.rs`, `device.rs` (GObject subclasses with properties)
- `src/ui/bluetooth/`: `page.rs`, `device_row.rs`, `device_page.rs`, `pairing_dialog.rs`
- `data/`: `.desktop` file, AppStream metainfo, icons, GResource with `.ui` or Blueprint files

## BlueZ library: zbus

I recommend talking to BlueZ over D-Bus with **zbus** and our own small typed proxies, rather than the higher-level **bluer** crate.

|  | zbus | bluer |
| --- | --- | --- |
| What it is | Pure-Rust D-Bus library; we define proxies for `org.bluez.Adapter1`, `Device1`, `Battery1`, `Agent1` | Official BlueZ bindings with a ready-made Bluetooth API |
| Async runtime | Runtime-agnostic; futures run directly on the GTK main loop via `glib::spawn_future_local` | Requires Tokio, so we would run a second runtime and pass messages to the UI thread |
| System deps | None beyond GTK | Links the C `libdbus` |
| Pairing agent | We implement `org.bluez.Agent1` as a zbus interface (about 100 lines) | Built in |
| Code we own | About 200 to 300 lines of proxy and model code | Less up front |
| Reuse later | Same library covers UPower, NetworkManager, udisks2 for future device types | Bluetooth only |

The deciding point is the event loop: zbus fits GTK's single-threaded model with no bridging, and it is the D-Bus library we will need anyway once Openhub grows past Bluetooth. bluer is the better choice only if we wanted GATT/BLE-level work (custom services, L2CAP sockets), which v0.1 does not.

## UI screens

The app is built as a sidebar-plus-content layout so other device categories slot in later without a redesign; for v0.1 the sidebar has one entry, Bluetooth.

| Screen | libadwaita building blocks | What it shows |
| --- | --- | --- |
| Main window | `AdwApplicationWindow`, `AdwNavigationSplitView`, `AdwToolbarView`, `AdwHeaderBar` | Sidebar of device categories, content pane for the selected one; collapses to one pane on narrow windows |
| Bluetooth page | `AdwPreferencesPage` with groups, `AdwSwitchRow` for power | Adapter power switch and name, then a "My Devices" group (paired) and a "Nearby Devices" group with a scanning spinner |
| Device row | `AdwActionRow` with icon, subtitle, suffix button | Name, type icon, "Connected" / "Not connected" / battery %, a Connect or Disconnect button |
| Device details | `AdwNavigationPage` pushed onto the content pane | Address, type, trusted/blocked switches, battery, Forget button with confirmation |
| Pairing dialog | `AdwAlertDialog` | "Confirm the code 123456 matches" or a PIN entry, driven by the pairing agent |
| Empty and error states | `AdwStatusPage` | No adapter found, Bluetooth service not running, adapter blocked |
| Feedback | `AdwToastOverlay` | Short messages such as "Couldn't connect to WH-1000XM4" |

The target minimum is GTK 4.14 and libadwaita 1.5, which matches Ubuntu 24.04 LTS; newer distros (Fedora, Arch, Debian 13) are already above that. `AdwAlertDialog` needs 1.5, so this floor is the lowest that keeps the modern dialog.

## Milestones

Each milestone ends with something CJ can run on a real machine; the cloud container has no Bluetooth hardware, so it can only prove the build.

1. **M0 Skeleton**: Cargo project, app ID, window with split view and empty Bluetooth page, `.desktop` file and icon placeholder, CI build on GitHub Actions.
2. **M1 Read-only list**: connect to BlueZ, show adapters and paired devices with live updates from `InterfacesAdded` / `PropertiesChanged`, empty and error states.
3. **M2 Control**: adapter power switch, scanning with the Nearby list, connect and disconnect with toasts on failure.
4. **M3 Pairing**: pairing agent, confirmation dialog, trust, forget, device details page with battery.
5. **M4 Packaging**: Flatpak manifest with the `org.bluez` system-bus permission, Meson or plain Cargo install, AppStream metadata, then a v0.1 tag.

After v0.1 the same pattern extends to the next device category (for example USB via udev, or batteries via UPower).

## Open questions

- [ ] App ID and name: is `io.github.<you>.Openhub` right, or do you own a domain to use?
- [ ] Where should the code live: a new GitHub repo (which account and name), or local only for now?
- [ ] Build system: plain Cargo (simpler) or Meson + Cargo (the GNOME convention, needed for translations and a polished Flatpak)? I lean to Cargo now, Meson at M4.
- [ ] Distribution target: Flatpak first, distro packages, or just `cargo run` for now?
- [ ] Is Ubuntu 24.04 (GTK 4.14, libadwaita 1.5) the oldest system you need to support?
- [ ] Should Openhub run its own pairing agent, or defer to the desktop's (GNOME Settings already registers one)?

## Decisions

CJ answered the open questions on 2026-10-13.

| Question | Decision |
| --- | --- |
| App ID | Decide later; `io.github.cjts15.Openhub` is a placeholder set in one place in `meson.build` |
| Repository | [CJTS15/openhub](https://github.com/CJTS15/openhub) |
| Build system | GNOME convention: Meson driving Cargo |
| Distribution | Flatpak, on the GNOME 50 runtime |
| Oldest system | Flatpak runs on any distro with Flatpak; native builds need GTK 4.14 / libadwaita 1.5 (Ubuntu 24.04), checked in CI |
| Pairing | Openhub runs its own pairing agent |