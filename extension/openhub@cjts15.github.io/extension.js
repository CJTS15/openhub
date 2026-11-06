// SPDX-License-Identifier: GPL-3.0-or-later

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GnomeBluetooth from 'gi://GnomeBluetooth?version=3.0';
import GObject from 'gi://GObject';
import Gvc from 'gi://Gvc';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import {BarLevel} from 'resource:///org/gnome/shell/ui/barLevel.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {getMixerControl} from 'resource:///org/gnome/shell/ui/status/volume.js';

import {batteries, bluetoothLabel} from './devices.js';

const APP_ID = 'io.github.cjts15.Openhub';

// Only the parts of UPower this extension reads.
const UPowerProxy = Gio.DBusProxy.makeProxyWrapper(`
<node>
  <interface name="org.freedesktop.UPower">
    <method name="EnumerateDevices"><arg type="ao" direction="out"/></method>
    <signal name="DeviceAdded"><arg type="o"/></signal>
    <signal name="DeviceRemoved"><arg type="o"/></signal>
  </interface>
</node>`);

const UPowerDeviceProxy = Gio.DBusProxy.makeProxyWrapper(`
<node>
  <interface name="org.freedesktop.UPower.Device">
    <property name="Type" type="u" access="read"/>
    <property name="Model" type="s" access="read"/>
    <property name="Serial" type="s" access="read"/>
    <property name="Percentage" type="d" access="read"/>
    <property name="PowerSupply" type="b" access="read"/>
    <property name="IsPresent" type="b" access="read"/>
  </interface>
</node>`);

const OpenhubIndicator = GObject.registerClass(
class OpenhubIndicator extends PanelMenu.Button {
    _init(extension) {
        super._init(0.5, 'Openhub');

        this._appIcon = Gio.icon_new_for_string(
            extension.dir.get_child('icons').get_child('openhub-symbolic.svg').get_path());
        this._icon = new St.Icon({gicon: this._appIcon, style_class: 'system-status-icon'});
        this.add_child(this._icon);

        this.menu.box.add_style_class_name('openhub-menu');
        this.menu.addMenuItem(headerItem(this._appIcon, () => {
                this.menu.close();
                showPage('system');
            }));

        const bluetooth = card({icon: 'bluetooth-active-symbolic', title: 'Bluetooth', chevron: true});
        bluetooth.icon.add_style_class_name('openhub-circle');
        this._circle = bluetooth.icon;

        // The shell theme's accent, read from its slider style: the accent
        // colour with GNOME's theme, or a custom theme's (such as one
        // Recoolor generates). -st-accent-color only follows GNOME's own
        // accent setting. Read again whenever the theme changes.
        this._accent = null;
        this._accentProbe = new St.Widget({style_class: 'slider', opacity: 0, width: 0, height: 0});
        this._accentProbe.connectObject('style-changed', () => this._applyAccent(), this);
        this.menu.box.add_child(this._accentProbe);
        bluetooth.item.connect('activate', () => showPage('bluetooth'));
        this._bluetooth = bluetooth.subtitle;
        this.menu.addMenuItem(bluetooth.item);

        this._batteries = new PopupMenu.PopupMenuSection();
        this.menu.addMenuItem(sectionHeader('battery-symbolic', 'Batteries',
            'Check battery levels of your devices', this._batteries));
        this.menu.addMenuItem(this._batteries);

        this._outputs = new PopupMenu.PopupMenuSection();
        this.menu.addMenuItem(sectionHeader('audio-speakers-symbolic', 'Sound Output',
            'Select your audio output device', this._outputs));
        this.menu.addMenuItem(this._outputs);
        this._outputItems = new Map();
        // GNOME Shell's own mixer connection, shared with its volume slider.
        this._mixer = getMixerControl();
        this._mixer.connectObject(
            'output-added', (c, id) => this._addOutput(id),
            'output-removed', (c, id) => this._removeOutput(id),
            'active-output-update', (c, id) => this._setActiveOutput(id),
            'state-changed', () => this._listOutputs(),
            this);
        this._listOutputs();

        // Batteries only Openhub reads (Razer, some 2.4 GHz dongles), while
        // Openhub runs; asked again each time the menu opens.
        // ponytail: no change signal, so the icon's low warning only counts
        // these after the menu opens; add a Changed signal if that matters.
        this._openhubBatteries = [];
        this._openhubWatch = Gio.bus_watch_name(Gio.BusType.SESSION, APP_ID,
            Gio.BusNameWatcherFlags.NONE,
            () => this._readOpenhubBatteries(),
            () => {
                this._openhubBatteries = [];
                this._queueSync();
            });
        this.menu.connectObject('open-state-changed', (menu, open) => {
            if (open)
                this._readOpenhubBatteries();
        }, this);

        this._devices = new Map();
        this._cancellable = new Gio.Cancellable();
        this._upower = new UPowerProxy(Gio.DBus.system, 'org.freedesktop.UPower',
            '/org/freedesktop/UPower', (proxy, error) => {
                if (error) {
                    console.error(`Openhub: UPower isn't available: ${error.message}`);
                    return;
                }
                this._signals = [
                    proxy.connectSignal('DeviceAdded', (p, sender, [path]) => this._addDevice(path)),
                    proxy.connectSignal('DeviceRemoved', (p, sender, [path]) => this._removeDevice(path)),
                ];
                this._upower.EnumerateDevicesAsync()
                    .then(([paths]) => paths.forEach(path => this._addDevice(path)))
                    .catch(e => console.error(`Openhub: ${e.message}`));
            }, this._cancellable);

        this._client = new GnomeBluetooth.Client();
        const store = this._client.get_devices();
        this._client.connectObject(
            'device-added', (c, device) => this._watchBluetooth(device),
            'device-removed', () => this._queueSync(), this);
        for (let i = 0; i < store.get_n_items(); i++)
            this._watchBluetooth(store.get_item(i));

        this._sync();
    }

    /**
     * Lists the outputs the mixer already knows: it was opened at login, so
     * their `output-added` signals came before this extension started.
     */
    _listOutputs() {
        if (this._mixer.get_state() !== Gvc.MixerControlState.READY)
            return;
        // ponytail: one entry per sink, for its current port; a sink's other
        // ports (such as speakers beside headphones) appear once they change.
        for (const stream of this._mixer.get_sinks()) {
            const device = this._mixer.lookup_device_from_stream(stream);
            if (device)
                this._addOutput(device.get_id());
        }
        const active = this._mixer.lookup_device_from_stream(this._mixer.get_default_sink());
        if (active)
            this._setActiveOutput(active.get_id());
    }

    _addOutput(id) {
        if (this._outputItems.has(id))
            return;
        const device = this._mixer.lookup_output_id(id);
        if (!device)
            return;
        // Gvc names full-colour icons (video-display, audio-speakers); the
        // menu uses their symbolic versions, with speakers as the fallback.
        const name = device.get_icon_name();
        const output = card({
            style: 'openhub-item',
            gicon: Gio.ThemedIcon.new_from_names(
                [name ? `${name}-symbolic` : '', 'audio-speakers-symbolic'].filter(Boolean)),
            title: device.description,
            subtitle: device.origin,
        });
        output.radio = new St.Icon({icon_name: 'radio-symbolic', style_class: 'popup-menu-icon openhub-radio'});
        output.item.add_child(output.radio);
        output.item.connect('activate', () => {
            const current = this._mixer.lookup_output_id(id);
            if (current)
                this._mixer.change_output(current);
        });
        this._outputs.addMenuItem(output.item);
        this._outputItems.set(id, output);
    }

    _removeOutput(id) {
        this._outputItems.get(id)?.item.destroy();
        this._outputItems.delete(id);
    }

    _setActiveOutput(activeId) {
        this._activeOutput = activeId;
        for (const [id, output] of this._outputItems) {
            const active = id === activeId;
            output.radio.icon_name = active ? 'radio-checked-symbolic' : 'radio-symbolic';
            if (active)
                output.item.add_style_class_name('openhub-selected');
            else
                output.item.remove_style_class_name('openhub-selected');
            output.item.set_style(active && this._accent ? `border-color: ${this._accent.color};` : null);
            output.radio.set_style(active && this._accent ? `color: ${this._accent.color};` : null);
        }
    }

    _applyAccent() {
        const color = this._accentProbe.get_theme_node().get_color('-barlevel-active-background-color');
        this._accent = accentStyle(color);
        this._circle.set_style(this._accent
            ? `background-color: ${this._accent.color}; color: ${this._accent.foreground};`
            : null);
        this._setActiveOutput(this._activeOutput);
    }

    _readOpenhubBatteries() {
        Gio.DBus.session.call(APP_ID, '/io/github/cjts15/Openhub',
            'io.github.cjts15.Openhub.Devices', 'GetBatteries', null,
            new GLib.VariantType('(a(sud))'), Gio.DBusCallFlags.NO_AUTO_START, 5000,
            this._cancellable, (bus, result) => {
                if (this._cancellable.is_cancelled())
                    return;
                try {
                    const [list] = bus.call_finish(result).deepUnpack();
                    this._openhubBatteries = list;
                } catch {
                    // An Openhub without this interface, or one that's quitting.
                    this._openhubBatteries = [];
                }
                this._queueSync();
            });
    }

    _addDevice(path) {
        if (this._devices.has(path))
            return;
        const device = new UPowerDeviceProxy(Gio.DBus.system, 'org.freedesktop.UPower', path,
            (proxy, error) => {
                if (error)
                    return;
                proxy.connectObject('g-properties-changed', () => this._queueSync(), this);
                this._queueSync();
            }, this._cancellable);
        this._devices.set(path, device);
    }

    _removeDevice(path) {
        this._devices.get(path)?.disconnectObject(this);
        this._devices.delete(path);
        this._queueSync();
    }

    _watchBluetooth(device) {
        device.connectObject('notify::connected', () => this._queueSync(), this);
        this._queueSync();
    }

    /** Redraws once for a burst of changes, such as a device's properties arriving together. */
    _queueSync() {
        if (this._syncId)
            return;
        this._syncId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._syncId = 0;
            this._sync();
            return GLib.SOURCE_REMOVE;
        });
    }

    _sync() {
        const rows = batteries([...this._devices.values()]
            .filter(d => d.Type !== null)
            .map(d => ({
                type: d.Type,
                model: d.Model ?? '',
                serial: d.Serial ?? '',
                percentage: d.Percentage ?? 0,
                powerSupply: d.PowerSupply ?? true,
                present: d.IsPresent ?? false,
            }))
            .concat(this._openhubBatteries.map(([model, type, percentage]) => ({
                type, model, serial: '', percentage, powerSupply: false, present: true,
            }))));

        this._batteries.removeAll();
        if (rows.length === 0) {
            const empty = card({style: 'openhub-item', icon: 'battery-missing-symbolic', title: 'No devices with a battery'});
            empty.item.reactive = false;
            this._batteries.addMenuItem(empty.item);
        }
        for (const row of rows)
            this._batteries.addMenuItem(batteryItem(row));

        // The icon warns while any device is low.
        const low = rows.find(row => row.low);
        if (low) {
            this._icon.gicon = null;
            this._icon.icon_name = 'battery-level-10-symbolic';
            this._icon.add_style_class_name('openhub-low');
        } else {
            this._icon.gicon = this._appIcon;
            this._icon.remove_style_class_name('openhub-low');
        }

        const store = this._client.get_devices();
        let connected = 0;
        for (let i = 0; i < store.get_n_items(); i++) {
            if (store.get_item(i).connected)
                connected++;
        }
        this._bluetooth.text = bluetoothLabel(connected);
    }

    destroy() {
        this._cancellable.cancel();
        Gio.bus_unwatch_name(this._openhubWatch);
        this._mixer.disconnectObject(this);
        this._accentProbe.disconnectObject(this);
        this._mixer = null;
        this._outputItems.clear();
        if (this._syncId)
            GLib.source_remove(this._syncId);
        this._syncId = 0;
        for (const id of this._signals ?? [])
            this._upower.disconnectSignal(id);
        for (const device of this._devices.values())
            device.disconnectObject(this);
        const store = this._client.get_devices();
        for (let i = 0; i < store.get_n_items(); i++)
            store.get_item(i).disconnectObject(this);
        this._client.disconnectObject(this);
        this._client = null;
        this._upower = null;
        this._devices.clear();
        super.destroy();
    }
});

/**
 * A box of children stacked vertically. A Clutter layout rather than
 * St.BoxLayout's `vertical`, which GNOME 48 renamed.
 *
 * @param {Clutter.Actor[]} children the contents
 * @param {object} params more St.Widget properties
 * @param {number} spacing the gap between children, in pixels
 * @returns {St.Widget} the box
 */
function column(children, params = {}, spacing = 0) {
    const box = new St.Widget({
        layout_manager: new Clutter.BoxLayout({orientation: Clutter.Orientation.VERTICAL, spacing}),
        y_align: Clutter.ActorAlign.CENTER,
        ...params,
    });
    children.forEach(child => box.add_child(child));
    return box;
}

/**
 * A rounded row: an icon in a tile, a title over a subtitle, and an
 * optional chevron.
 *
 * @param {{icon: string, gicon?: Gio.Icon, title: string, subtitle?: string,
 *   chevron?: boolean, style?: string}} params the contents: `style` is
 *   'openhub-card' for a row with a background, 'openhub-item' for one
 *   inside a section
 * @returns {{item: PopupMenu.PopupBaseMenuItem, icon: St.Bin, title: St.Label,
 *   subtitle: St.Label, text: St.Widget}} the row and its parts
 */
function card({icon, gicon, title, subtitle = '', chevron = false, style = 'openhub-card'}) {
    const item = new PopupMenu.PopupBaseMenuItem({style_class: style});
    const tile = new St.Bin({
        style_class: 'openhub-tile',
        y_align: Clutter.ActorAlign.CENTER,
        // Only one of gicon and icon_name: setting icon_name clears gicon.
        child: new St.Icon({...(gicon ? {gicon} : {icon_name: icon}), icon_size: 20}),
    });
    item.add_child(tile);
    const titleLabel = new St.Label({text: title, style_class: 'openhub-card-title'});
    const subtitleLabel = new St.Label({text: subtitle, style_class: 'openhub-card-subtitle'});
    subtitleLabel.visible = subtitle !== '';
    subtitleLabel.connect('notify::text', () => (subtitleLabel.visible = subtitleLabel.text !== ''));
    const text = column([titleLabel, subtitleLabel], {x_expand: true});
    item.add_child(text);
    if (chevron) {
        item.add_child(new St.Icon({
            icon_name: 'go-next-symbolic',
            style_class: 'popup-menu-icon',
            y_align: Clutter.ActorAlign.CENTER,
        }));
    }
    return {item, icon: tile, title: titleLabel, subtitle: subtitleLabel, text};
}

/**
 * The menu's first row: Openhub's icon and name, and a settings button
 * that opens Openhub.
 *
 * @param {Gio.Icon} appIcon Openhub's symbolic icon, as in the top bar
 * @param {Function} openSettings called by the settings button
 * @returns {PopupMenu.PopupBaseMenuItem} the item
 */
function headerItem(appIcon, openSettings) {
    const item = new PopupMenu.PopupBaseMenuItem({
        reactive: false,
        can_focus: false,
        style_class: 'openhub-header',
    });
    // Non-clickable items are greyed out, which isn't wanted for a title.
    item.remove_style_pseudo_class('insensitive');
    item.add_child(new St.Icon({gicon: appIcon, icon_size: 32, y_align: Clutter.ActorAlign.CENTER}));
    item.add_child(column([
        new St.Label({text: 'Openhub', style_class: 'openhub-title'}),
        new St.Label({text: 'Connected Devices & Settings', style_class: 'openhub-card-subtitle'}),
    ], {x_expand: true}));
    const settings = new St.Button({
        style_class: 'icon-button openhub-settings',
        can_focus: true,
        accessible_name: 'Open Openhub',
        y_align: Clutter.ActorAlign.CENTER,
        child: new St.Icon({icon_name: 'emblem-system-symbolic', icon_size: 16}),
    });
    settings.connect('clicked', openSettings);
    item.add_child(settings);
    return item;
}

/**
 * A section's header: an icon, title and subtitle, and a chevron that
 * hides or shows the section below.
 *
 * @param {string} icon the icon name
 * @param {string} title the title
 * @param {string} subtitle what the section is for
 * @param {PopupMenu.PopupMenuSection} section the rows it collapses
 * @returns {PopupMenu.PopupBaseMenuItem} the item
 */
function sectionHeader(icon, title, subtitle, section) {
    const header = card({icon, title, subtitle});
    header.item.add_style_class_name('openhub-section');
    const chevron = new St.Icon({
        icon_name: 'pan-up-symbolic',
        style_class: 'popup-menu-icon',
        y_align: Clutter.ActorAlign.CENTER,
    });
    header.item.add_child(chevron);
    // Toggles in place instead of activating, which would close the menu.
    header.item.activate = () => {
        section.actor.visible = !section.actor.visible;
        chevron.icon_name = section.actor.visible ? 'pan-up-symbolic' : 'pan-down-symbolic';
    };
    return header.item;
}

/**
 * A battery row: the device's icon, its name over "Connected", and its
 * percentage over a level bar. It opens Openhub's Power page.
 *
 * @param {{name: string, icon: string, percentage: number, low: boolean}} row the device
 * @returns {PopupMenu.PopupBaseMenuItem} the item
 */
function batteryItem(row) {
    const {item, text} = card({style: 'openhub-item', icon: row.icon, title: row.name});
    item.connect('activate', () => showPage('power'));

    const status = new St.BoxLayout({style_class: 'openhub-status'});
    status.add_child(new St.Widget({style_class: 'openhub-dot', y_align: Clutter.ActorAlign.CENTER}));
    status.add_child(new St.Label({text: 'Connected', style_class: 'openhub-card-subtitle'}));
    text.add_child(status);

    const percent = new St.Label({
        text: `${row.percentage}%`,
        style_class: 'openhub-percent',
        x_align: Clutter.ActorAlign.END,
    });
    // GNOME Shell's own level bar, styled like its sliders, so it takes the
    // shell theme's colours (the accent colour, or a custom theme's).
    const bar = new BarLevel({style_class: 'slider openhub-level'});
    bar.value = Math.min(row.percentage, 100) / 100;
    if (row.low) {
        percent.add_style_class_name('openhub-low');
        bar.add_style_class_name('openhub-low');
    }
    item.add_child(column([percent, bar], {}, 6));
    return item;
}

/**
 * The accent as inline CSS, with a readable colour for an icon on it.
 *
 * @param {Clutter.Color|Cogl.Color|null} color the theme's accent
 * @returns {{color: string, foreground: string}|null} CSS colours, or null
 *   when the theme has none
 */
function accentStyle(color) {
    if (!color || color.alpha === 0)
        return null;
    const {red, green, blue} = color;
    // Relative luminance, roughly: light accents get a dark icon.
    const light = (0.299 * red + 0.587 * green + 0.114 * blue) / 255 > 0.6;
    return {color: `rgb(${red}, ${green}, ${blue})`, foreground: light ? '#1e1e2e' : '#ffffff'};
}

/**
 * Opens Openhub on a page ('' for the page it was on). Openhub's
 * `show-page` action does this when it's running; otherwise Openhub starts
 * first, and the page is shown once its D-Bus name appears.
 *
 * @param {string} page a sidebar page, such as 'bluetooth'
 */
function showPage(page) {
    const activate = launched => Gio.DBus.session.call(APP_ID, '/io/github/cjts15/Openhub',
        'org.gtk.Actions', 'Activate',
        new GLib.Variant('(sava{sv})', ['show-page', [new GLib.Variant('s', page)], {}]),
        null, Gio.DBusCallFlags.NO_AUTO_START, 5000, null, (bus, result) => {
            try {
                bus.call_finish(result);
            } catch {
                // An Openhub without `show-page` still opens; it just stays on its page.
                if (!launched)
                    launch();
            }
        });
    const launch = () => {
        const app = Shell.AppSystem.get_default().lookup_app(`${APP_ID}.desktop`);
        if (!app) {
            Main.notify('Openhub', 'Openhub isn\'t installed.');
            return;
        }
        app.activate();
        if (!page)
            return;
        const watch = Gio.bus_watch_name(Gio.BusType.SESSION, APP_ID, Gio.BusNameWatcherFlags.NONE,
            () => {
                Gio.bus_unwatch_name(watch);
                activate(true);
            }, null);
    };
    activate(false);
}

export default class OpenhubExtension extends Extension {
    enable() {
        this._indicator = new OpenhubIndicator(this);
        Main.panel.addToStatusArea(this.uuid, this._indicator);
    }

    disable() {
        this._indicator?.destroy();
        this._indicator = null;
    }
}
