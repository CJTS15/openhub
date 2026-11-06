# Openhub GNOME Shell extension

An icon in the top bar, two switches like macOS's Control Center, with a menu of your devices:

- the battery of every mouse, keyboard, headset and other peripheral UPower knows, lowest first, with a warning below 20%;
- how many Bluetooth devices are connected, which opens Openhub's Bluetooth page;
- the sound outputs, with the default checked; picking one makes it the default;
- a shortcut into Openhub.

The icon turns into a low-battery icon while any device is below 20%.

It works on its own, without Openhub running. It reads UPower, BlueZ and the sound server the same way GNOME Shell does, and only opens Openhub when you click a row. While Openhub runs, the menu also lists the batteries only Openhub reads (Razer mice, some 2.4 GHz dongles), from Openhub's `GetBatteries` D-Bus call.

Supported: GNOME 47 to 51: the Bluetooth icon and the selected output use the accent colour, which GNOME 47 introduced. The level bars use the shell theme's colours, so they also follow custom shell themes.

## Install

From the repository root:

```sh
gnome-extensions pack extension/openhub@cjts15.github.io --force \
    --extra-source=devices.js --extra-source=icons
gnome-extensions install --force openhub@cjts15.github.io.shell-extension.zip
```

Log out and back in once, so GNOME Shell loads it, then turn it on:

```sh
gnome-extensions enable openhub@cjts15.github.io
```

Run the same two install commands again after updating.

## Try it without touching your session

```sh
gnome-shell --devkit
```

This opens a separate GNOME Shell in a window (GNOME 49 and later). Installed extensions are loaded there too.

## Test

```sh
gjs -m extension/tests/devices.test.js
```

## Files

- `extension.js`: the top-bar icon and its menu
- `devices.js`: the menu's rules (which batteries to list, the labels), without GNOME Shell imports, so they can be tested with `gjs`
- `stylesheet.css`: the level bars and the warning colour
- `icons/openhub-symbolic.svg`: the top-bar icon

The extension doesn't import anything from Openhub. It calls Openhub's `app.show-page` action over D-Bus, so this folder can move to its own repository.

License: GPL-3.0-or-later.
