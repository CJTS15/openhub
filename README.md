# Openhub

A lightweight GTK app that manages all connected devices, built with GTK4 + libadwaita in Rust. It starts with Bluetooth.

## Status

[v0.1.0](https://github.com/CJTS15/openhub/releases/tag/v0.1.0) is out and covers Bluetooth: turn the adapter on and off, connect and disconnect devices, see their battery level, scan for and pair new devices through Openhub's own pairing agent, and trust, block or forget them.

Next is v0.2, which adds a USB page: a live list of USB devices with details, and safe eject for drives.

## Plans

- [PLAN_V1.md](PLAN_V1.md): v0.1, Bluetooth (milestones M0 to M4, released)
- [PLAN_V2.md](PLAN_V2.md): v0.2, USB devices (milestones M5 to M8)

## Installing

Download `openhub-0.1.0-x86_64.flatpak` from the [v0.1.0 release](https://github.com/CJTS15/openhub/releases/tag/v0.1.0). It needs the GNOME 50 runtime from Flathub:

```sh
flatpak install --user flathub org.gnome.Platform//50
flatpak install --user openhub-0.1.0-x86_64.flatpak
flatpak run io.github.cjts15.Openhub
```

Known issue: with blueman-applet running, turning the adapter off and on can freeze GNOME Shell through a flood of connection notifications ([#4](https://github.com/CJTS15/openhub/issues/4)). Stopping or removing blueman avoids it.

## Building

### Flatpak (recommended)

Requires `flatpak-builder` (from your distribution, or `flatpak install flathub org.flatpak.Builder` and run it as `flatpak run org.flatpak.Builder`) and the GNOME 50 SDK with the Rust extension:

```sh
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
flatpak-builder --user --install --force-clean _flatpak build-aux/io.github.cjts15.Openhub.json
flatpak run io.github.cjts15.Openhub
```

The manifest currently lets the build fetch crates over the network. Offline vendored sources are added before a Flathub submission.

### Native (Meson)

Needs GTK >= 4.14, libadwaita >= 1.5 (Ubuntu 24.04, Fedora 40, Debian 13 or newer), Meson, and a Rust toolchain:

```sh
meson setup _build
meson compile -C _build
meson test -C _build
./_build/src/openhub
```

For quick iteration, `cargo run` also works.

## Architecture

- `src/application.rs`: `AdwApplication` subclass, app actions and shortcuts
- `src/window.rs` + `data/resources/ui/window.ui`: main window with a category sidebar and content pane
- `src/config.rs`: app ID and version, passed in by Meson
- `src/bluez/`: zbus proxies, the pairing agent and `BluetoothManager`, the only code that talks to BlueZ over D-Bus
- `src/model/`: `Adapter` and `Device` GObjects the UI binds to
- `src/ui/bluetooth/` + `data/resources/ui/bluetooth_page.ui`: the Bluetooth page, device rows, and the pairing and device details dialogs

## License

GPL-3.0-or-later
