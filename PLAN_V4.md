# Openhub v0.4 plan: Mouse, Keyboard and Audio

Oct 15, 2026 · @CJ

## Summary

v0.4 adds three pages (Mouse, Keyboard, Audio) and ships every feature you asked for, but only where the device itself exposes a way to change it. Linux has no common API for DPI, button mapping or keyboard lighting; each comes from a vendor protocol, so support depends on the exact model.

Defaults, updated with CJ's answers on 2026-10-15:

- **Mouse:** Logitech mice (your MX Master 3S included) through Logitech's HID++ protocol directly, the way OpenLogi does: DPI, SmartShift, battery. Other brands through ratbagd, the daemon Piper is built on: DPI, report rate and button shortcuts where the mouse accepts them. Button remapping for Logitech mice comes later.
- **Keyboard:** lighting and key remap through VIA over the USB cable, stored on the keyboard. Every keyboard gets a Keyboard Shortcuts button that opens GNOME Settings.
- **Audio:** one page with Output and Input sections, like GNOME Settings > Sound, and a built-in equalizer for outputs. Opening EasyEffects or another EQ app moves to v0.5.
- **Cut from v0.4:** Logitech button remapping, remap for non-VIA keyboards, OpenRGB lighting, EQ app integration, and pointer speed (GNOME Settings already has it).
- **Milestones:** M13 to M18. The built-in EQ gets its own milestone because it is now the largest piece of v0.4.

## Feasibility

Each feature you asked for, the route Linux offers, and whether it survives the Flatpak sandbox. "In v0.4" is my recommendation.

| Feature | Route | Works on | Flatpak | In v0.4 |
| --- | --- | --- | --- | --- |
| Mouse DPI, Logitech | HID++ over `/dev/hidraw*`, as [OpenLogi](https://github.com/AprilNEA/OpenLogi) and Solaar do | Logitech mice on Bolt, Unifying, Bluetooth or cable, MX Master 3S included | Covered by `--device=all` (already accepted for VIA) and a host udev rule | Yes |
| Mouse DPI, other brands | [ratbagd](https://github.com/libratbag/libratbag) on the system bus | SteelSeries, Roccat, some ASUS and others; not generic mice | One `--system-talk-name`; ratbagd must be installed on the host | Yes |
| Mouse button shortcuts, other brands | ratbagd button mapping | Same list, where the mouse accepts writes | Same | Yes, where it works |
| Mouse button remap, Logitech | Software remap by a background agent (evdev grab and uinput) | Logitech mice, only while the agent runs | Needs a background service | No, later |
| Keyboard lighting | VIA raw HID (QMK firmware), cable only | VIA/QMK boards, including the Yunzii AL68 | Needs `--device=all` and a host udev rule | Yes |
| Keyboard lighting, other brands | [OpenRGB](https://openrgb.org) SDK over TCP | Whatever OpenRGB supports, only while its server runs | Needs `--share=network` | No, later |
| Key remap | VIA dynamic keymap, stored on the keyboard | VIA/QMK boards | Same as VIA lighting | Yes |
| Keyboard shortcuts | Open GNOME Settings > Keyboard > Keyboard Shortcuts | GNOME | Needs a check (see Keyboard page) | Yes |
| Key remap, any keyboard | hwdb, keyd or input-remapper | All keyboards | Needs root or a host daemon; not from the sandbox | No, later |
| Built-in equalizer | PipeWire filter-chain, saved as a config drop-in | PipeWire 0.3.x and later | Needs `xdg-config/pipewire` access and a PipeWire restart | Yes |
| Open an EQ app | Hand off to [EasyEffects](https://github.com/wwmm/easyeffects) | Any PipeWire system | Launching another app is limited | No, v0.5 |

CJ confirmed that VIA on the AL68 works only over the USB cable, not through the 2.4 GHz dongle.

## Mouse page

The Mouse page lists every pointing device and opens a settings dialog for each one Openhub can drive. Logitech mice go through HID++ directly (CJ's decision, 2026-10-15); other brands go through ratbagd over D-Bus (`org.freedesktop.ratbag1`) with zbus, the same way Openhub talks to BlueZ and UPower.

### Piper or ratbagd

Piper is not an alternative to ratbagd; it is a GTK app built on top of it. Piper sends every change to ratbagd, and ratbagd writes it to the mouse. So both routes support exactly the same mice and features.

|  | Use ratbagd directly | Open Piper |
| --- | --- | --- |
| Mice supported | ratbagd's list | The same list |
| Feels like Openhub | Yes, own dialog | No, a second app |
| Cost when idle | None; ratbagd sleeps until asked | None |
| Cost when opened | A few D-Bus calls; first read of a wireless mouse can take about a second | Same, plus starting Piper |
| Work for us | A D-Bus client, as with BlueZ and UPower | A launcher, with the same Flatpak limits as EasyEffects |
| On the user's system | ratbagd | ratbagd and Piper |

For non-Logitech mice, Openhub uses ratbagd directly. It is efficient: nothing runs or polls while the page is closed. Neither route works for keyboards in practice; ratbagd covers mice and a few Logitech gaming keyboards, not the AL68.

### Your MX Master 3S

ratbagd has a device file for the MX Master 3S, and DPI changes work. Button remapping on the 3S fails inside ratbagd today with HID++ errors, according to [Piper issue #988](https://github.com/libratbag/piper/issues/988); one commenter there describes libratbag as lightly maintained. So on your mouse, v0.4 can change DPI but probably not buttons. Logitech's own button features on Linux come from [Solaar](https://wiki.nixos.org/wiki/Logitech_MX_Master) or LogiOps, which remap in software while they run, not on the mouse.

### OpenLogi

[OpenLogi](https://github.com/AprilNEA/OpenLogi) is a Rust replacement for Logitech Options+ on Linux, macOS and Windows (MIT or Apache-2.0, about 22k stars, marked "not yet stable"). It skips ratbagd and speaks Logitech's HID++ protocol itself, using Solaar as its protocol reference.

- **Parts:** a GPUI desktop app, a background agent (`openlogi-agent`, a systemd user service on Linux) that owns all device I/O, and a CLI. Settings live in one TOML file.
- **Device side:** the agent opens `/dev/hidraw*` and sends HID++ commands over Bolt, Unifying, Bluetooth or USB: DPI (feature 0x2201), SmartShift (0x2111), scroll inversion (0x2121), battery, and static keyboard lighting (0x8070/0x8080). The HID++ code is a vendored fork of the `hidpp` crate (0BSD).
- **Buttons:** remapped in software "via the OS input hook", not stored on the mouse. Its udev rules grant access to `/dev/uinput` and the mouse's `/dev/input/event*`, so on Linux it most likely grabs the mouse's events and re-emits them through a virtual device (inferred; the README does not spell it out). This only works while the agent runs.
- **Limits:** only one app can own a receiver's HID++ channel, so it conflicts with Options+, and probably with Solaar or Openhub talking HID++ at the same time. Per-app profiles work on X11 only. No Flatpak.

Decision (CJ, 2026-10-15): Openhub borrows OpenLogi's device side and speaks HID++ itself for Logitech mice. It does not copy the background agent, so Logitech button remapping waits for a later release. Talking to OpenLogi's agent is not an option; its interface between app and agent is not documented.

- **List:** every mouse and touchpad from `/sys/class/input`, named and merged with its Bluetooth or USB entry. Battery comes from the existing UPower service.
- **Logitech (HID++):** DPI with presets (feature 0x2201), SmartShift mode and sensitivity (0x2111), scroll direction (0x2121), battery. Openhub keeps its own HID++ code in `src/hidpp/`, using the 0BSD `hidpp` crate as reference or dependency, and Solaar's protocol notes.
- **Other brands (ratbagd):** DPI steps and range, report rate (125 to 1000 Hz), button shortcuts (another button, a key combination, or a special action such as DPI cycle), onboard profiles. If ratbagd refuses a button write, buttons go read-only with a note.
- **Apply:** changes are held until Apply, then sent in one write.
- **Re-apply:** HID++ settings may reset when a Logitech mouse sleeps or reconnects (untested). If they do, Openhub re-applies saved settings when it sees the mouse return, but only while Openhub is running. M16 tests this on your 3S first.
- **One owner at a time:** only one app can talk HID++ to a receiver. When Options+, Solaar or OpenLogi is running, the dialog says so and stays read-only.
- **Unsupported mice:** a plain row with "This mouse has no settings Openhub can change". When ratbagd is missing, non-Logitech mice get one banner saying to install `ratbagd`.

Pointer speed, acceleration and natural scrolling stay in GNOME Settings; they are system settings, not device settings.

## Keyboard page

The Keyboard page lists every keyboard and opens lighting and remap settings for boards that speak VIA. The Yunzii AL68 is sold as a [QMK/VIA keyboard](https://the-gadgeteer.com/?p=458882), so it is the reference device.

VIA is a small command set over a raw HID interface (usage page 0xFF60). Openhub opens `/dev/hidraw*` for that interface and sends the same commands the VIA web app does. No daemon is involved.

- **Lighting:** brightness, effect, effect speed and colour (hue and saturation). Only the controls the firmware reports are shown. Per-key colour is out of v0.4.
- **Key remap:** pick a key by what it does now ("Caps Lock"), then pick what it should do ("Escape"). Openhub reads the keymap from the board, so no layout picture is needed. Layer 0 only in v0.4; layers 1 to 3 later.
- **Storage:** VIA writes to the keyboard's EEPROM, so the change follows the keyboard to other computers and survives a reinstall.
- **Reset:** a "Restore default keymap" button, with a confirmation, since a bad remap can make a key unusable.
- **Other keyboards:** a plain row with name, connection and battery, plus "This keyboard has no settings Openhub can change".

VIA on the AL68 works only over the USB cable (confirmed by CJ). When the board is on the dongle or Bluetooth, the dialog shows its settings greyed out with "Connect with a USB cable to change lighting and keys". One thing still needs a test in M17: whether Yunzii's firmware uses stock VIA lighting commands or a custom channel.

**Keyboard Shortcuts button:** every keyboard, VIA or not, gets a row that opens GNOME Settings at the Keyboard panel, where Keyboard Shortcuts lives. From Flatpak this goes through GNOME Settings' D-Bus `launch-panel` action, which needs `--talk-name=org.gnome.Settings`. I expect it lands on the Keyboard panel, one click from Keyboard Shortcuts; opening the shortcuts list directly needs a check.

On Linux, `/dev/hidraw*` is root-only until a udev rule grants it to the logged-in user. VIA users already need this rule. Openhub cannot install it from the sandbox, so the dialog shows the one-line rule and the command to add it when access is denied.

## Audio page

The Audio page has an Output section and an Input section, laid out like GNOME Settings > Sound, and a built-in equalizer for each output. Openhub reads devices and volume through the PulseAudio protocol, which PipeWire serves as `pipewire-pulse`; the equalizer talks to PipeWire directly.

- **Output:** speakers, headphones, earbuds, IEMs and USB DACs, with the default one marked, Set as default, volume and mute. Bluetooth outputs show their profile and codec (SBC, AAC, aptX, LDAC); battery comes from UPower.
- **Input:** microphones with default, volume, mute and a live level meter. The meter only runs while the page is visible.
- **Equalizer:** an Equalizer row on each output opens the EQ dialog for that device.

### Built-in equalizer

Each output gets its own EQ, saved per device, so your IEMs and speakers keep different curves. It needs PipeWire; on PulseAudio-only systems the row is hidden.

- **Graphic mode:** 10 bands from 31 Hz to 16 kHz, plus a preamp to avoid clipping, and a few presets (Flat, Bass boost, Vocal).
- **Parametric import:** load an [AutoEQ](https://github.com/jaakkopasanen/AutoEq) `ParametricEQ.txt` for your headphone or IEM model. That file is a short list of peak and shelf filters, which map one to one onto PipeWire's filters.
- **On and off:** one switch per device; off sends audio straight to the device again.

How it works: the EQ is a PipeWire filter-chain of biquad filters (`bq_peaking`, `bq_lowshelf`, `bq_highshelf`). It appears as a virtual output, "Openhub EQ: \<device>", that plays into the real device, and Openhub makes it the default while the EQ is on.

To keep working when Openhub is closed, Openhub saves the chain as a config file in `~/.config/pipewire/pipewire.conf.d/`, which PipeWire loads at login. Slider changes apply live through the filter's controls and are written back to that file. The catch is the first time an EQ is turned on: PipeWire only reads new config files at start, and Openhub cannot restart PipeWire from the sandbox. So on first use Openhub also runs the chain itself until the next login, when PipeWire takes over. I have not built this hand-over before; M15 starts by proving it.

Opening EasyEffects or another EQ app moves to v0.5.

## Architecture and sandbox

Each page gets its own service module beside `src/bluez/`, `src/usb/` and `src/upower/`, plus a page under `src/ui/`. The services run on the GTK main loop like the existing ones.

| Module | Talks to | New dependency |
| --- | --- | --- |
| `src/input/` | `/sys/class/input` (read-only, already visible in Flatpak) to find mice and keyboards | None |
| `src/hidpp/` | Logitech mice over `/dev/hidraw*`: HID++ 2.0 feature lookup, DPI, SmartShift, scroll, battery; Bolt and Unifying receivers and Bluetooth | Possibly the `hidpp` crate (0BSD); else our own small implementation |
| `src/ratbag/` | ratbagd over the system bus with zbus, for non-Logitech mice | None |
| `src/via/` | `/dev/hidraw*` with plain reads and writes; device found through sysfs | None (no hidapi) |
| `src/audio/` | pipewire-pulse through libpulse, for outputs, inputs, volume and the level meter | `libpulse-binding` and `libpulse-glib-binding`; libpulse is in the GNOME runtime |
| `src/eq/` | PipeWire directly, for the filter-chain and its live controls; writes the config drop-in | `pipewire` crate; needs libclang at build time (the Flatpak SDK's LLVM extension) |

Manifest additions, six in total:

| finish-arg | For | Cost |
| --- | --- | --- |
| `--system-talk-name=org.freedesktop.ratbag1` | Mouse settings | Low; Piper's Flatpak uses the same |
| `--socket=pulseaudio` | Audio lists, volume, meter | Low; standard for audio apps |
| `--filesystem=xdg-run/pipewire-0` | EQ talks to PipeWire | Low; EasyEffects' Flatpak uses the same |
| `--filesystem=xdg-config/pipewire:create` | EQ config drop-in | Medium; write access to one config folder |
| `--talk-name=org.gnome.Settings` | Keyboard Shortcuts button | Low |
| `--device=all` | VIA and Logitech HID++ through `/dev/hidraw*` | High; Flatpak has no narrower permission for hidraw, and Flathub will ask why. Accepted for now (CJ) |

If `--device=all` becomes a problem for Flathub, the fallback is VIA in native builds only. CI tests ratbagd with a python-dbusmock template, VIA with a fake hidraw replay, and audio and EQ against a headless PipeWire.

## Milestones

v0.4 starts after v0.3.0 is tagged (M12) and runs in order of risk, lowest first, so each page can ship on its own.

1. **M13 Three pages, read-only.** Mouse, Keyboard and Audio entries in the sidebar; device lists from `/sys/class/input` and pipewire-pulse, merged with Bluetooth, USB and UPower data. No settings yet.
2. **M14 Audio Output and Input.** Default device, volume, mute, Bluetooth codec, input level meter.
3. **M15 Built-in equalizer.** Starts by proving the hand-over from Openhub's own chain to PipeWire's config file. Then 10-band EQ, preamp, presets, AutoEQ import, per-device storage.
4. **M16 Mouse.** Starts by testing on your MX Master 3S whether HID++ settings survive sleep and reconnect. Then HID++ for Logitech (DPI, SmartShift, scroll, battery), ratbagd for other brands (DPI, report rate, buttons, profiles), Apply, and the one-owner check.
5. **M17 Keyboard.** VIA lighting and layer-0 remap over cable, restore defaults, udev rule hint, and the Keyboard Shortcuts button for every keyboard.
6. **M18 Packaging and v0.4.0.** Manifest additions, udev rules documented for hidraw, metainfo release notes, screenshots, tag.

After v0.4: opening EasyEffects or other EQ apps (v0.5), remap for any keyboard, OpenRGB lighting, mouse macros and keyboard layers 1 to 3.

## Open questions

- None right now. Every question so far is answered in Decisions below.

## Decisions

CJ answered the first round on 2026-10-15.

| Question | Decision |
| --- | --- |
| Built-in EQ or EasyEffects first | Built-in EQ in v0.4; EQ app integration later |
| `--device=all` for VIA | Accept for now and see how it goes |
| Mouse to test with | Logitech MX Master 3S; compare Piper and ratbagd (done above) |
| Remap on VIA keyboards only | Yes, plus a button to GNOME Settings > Keyboard > Keyboard Shortcuts |
| Audio inputs | One page with separate Output and Input sections, like GNOME Settings > Sound |
| VIA over the dongle | No, cable only |
| Logitech mice (after checking OpenLogi) | HID++ directly for Logitech (DPI, SmartShift, battery); ratbagd for other brands; Logitech button remap later |
| EQ on first use | Yes: the EQ works at once, hosted by Openhub until the next login, then by PipeWire. Native builds restart PipeWire instead, so it is permanent at once. |
