# Openhub

A lightweight GTK app that manages all connected devices, built with GTK4 + libadwaita in Rust. It covers Bluetooth and USB.

## Status

[v0.2.0](https://github.com/CJTS15/openhub/releases/tag/v0.2.0) is the latest release.

- **Bluetooth** (since v0.1.0): turn the adapter on and off; connect and disconnect devices, with connected ones listed first; see battery levels; scan for and pair new devices through Openhub's own pairing agent; trust, block or forget devices.
- **USB** (new in v0.2.0): a live list of USB devices, grouped under the hubs they're connected through, with built-in devices hidden behind a toggle; a details dialog for each device; safe eject for USB drives.

## Plans

- [PLAN_V1.md](PLAN_V1.md): v0.1, Bluetooth (milestones M0 to M4, released)
- [PLAN_V2.md](PLAN_V2.md): v0.2, USB devices (milestones M5 to M8, released)

## Installing

Download `openhub-0.2.0-x86_64.flatpak` from the [v0.2.0 release](https://github.com/CJTS15/openhub/releases/tag/v0.2.0). It needs the GNOME 50 runtime from Flathub:

```sh
flatpak install --user flathub org.gnome.Platform//50
flatpak install --user openhub-0.2.0-x86_64.flatpak
flatpak run io.github.cjts15.Openhub
```

If blueman-applet is running, Openhub turns off its connection notifications, which can freeze GNOME Shell when many devices reconnect at once ([#4](https://github.com/CJTS15/openhub/issues/4)).

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
- `src/window.rs` + `data/resources/ui/window.ui`: main window; the sidebar switches the content pane between categories
- `src/config.rs`: app ID and version, passed in by Meson
- `src/bluez/`: zbus proxies, the pairing agent and `BluetoothManager`, the only code that talks to BlueZ over D-Bus
- `src/model/`: `Adapter` and `Device` GObjects the UI binds to
- `src/ui/bluetooth/` + `data/resources/ui/bluetooth_page.ui`: the Bluetooth page, device rows, and the pairing and device details dialogs
- `src/usb/`: the sysfs scan, `usb.ids` names, `UsbManager`, and safe eject through UDisks2; the only code that reads or controls USB devices
- `src/model/usb_device.rs`: the `UsbDevice` GObject; a hub holds its children
- `src/ui/usb/` + `data/resources/ui/usb_page.ui`: the USB page
- `tests/fixtures/sysfs-usb/`: a fake sysfs tree the USB parser tests run against

## License

GPL-3.0-or-later
