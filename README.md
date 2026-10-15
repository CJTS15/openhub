# Openhub

A lightweight GTK app that manages all connected devices, built with GTK4 + libadwaita in Rust. It covers Bluetooth, USB and power.

## Status

[v0.3.0](https://github.com/CJTS15/openhub/releases/tag/v0.3.0) is the latest release.

- **Bluetooth** (since v0.1.0): turn the adapter on and off; connect and disconnect devices, with connected ones listed first; see battery levels; scan for and pair new devices through Openhub's own pairing agent; trust, block or forget devices.
- **USB** (since v0.2.0): a live list of USB devices, grouped under the hubs they're connected through, with built-in devices hidden behind a toggle; a details dialog for each device; safe eject for USB drives.
- **Power** (new in v0.3.0): the laptop's battery and charging state, and every wireless device's battery UPower knows about, updated live; battery details with health and charge cycles; the "Preserve Battery Health" charge limit where supported; receiver batteries on the USB page.

Next is v0.4, which adds Mouse, Keyboard and Audio pages ([PLAN_V4.md](PLAN_V4.md)). So far (M13) they list every mouse, touchpad and keyboard, and every audio output and input, with connection, battery and the default audio devices; settings come in later milestones.

## Plans

- [PLAN_V1.md](PLAN_V1.md): v0.1, Bluetooth (milestones M0 to M4, released)
- [PLAN_V2.md](PLAN_V2.md): v0.2, USB devices (milestones M5 to M8, released)
- [PLAN_V3.md](PLAN_V3.md): v0.3, Power (milestones M9 to M12, released)
- [PLAN_V4.md](PLAN_V4.md): v0.4, Mouse, Keyboard and Audio (milestones M13 to M18)

## Installing

Download `openhub-0.3.0-x86_64.flatpak` from the [v0.3.0 release](https://github.com/CJTS15/openhub/releases/tag/v0.3.0). It needs the GNOME 50 runtime from Flathub:

```sh
flatpak install --user flathub org.gnome.Platform//50
flatpak install --user openhub-0.3.0-x86_64.flatpak
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
- `src/upower/`: UPower proxies, plain conversion rules, and `PowerManager`, the only code that talks to UPower
- `src/model/power_device.rs`: the `PowerDevice` GObject
- `src/ui/power/` + `data/resources/ui/power_page.ui`: the Power page
- `src/input/`: finds mice, touchpads and keyboards in `/sys/class/input`, one entry per physical device
- `src/audio/`: `AudioManager`, which lists audio outputs and inputs through libpulse (PipeWire serves it as `pipewire-pulse`)
- `src/ui/input/`, `src/ui/audio/`: the Mouse and Keyboard pages (one `InputPage` class) and the Audio page
- `src/ui/mod.rs`: `rescan_while_mapped`, the 2-second rescan the USB, Mouse and Keyboard pages share
- `src/watch.rs`: keeps a GObject in step with a cached D-Bus proxy's properties (shared by the BlueZ and UPower services)
- `tests/fixtures/sysfs-usb/`: a fake sysfs tree the USB parser tests run against
- `build-aux/test-upower.sh`: runs the UPower test against python-dbusmock (CI runs it too)

## License

GPL-3.0-or-later
