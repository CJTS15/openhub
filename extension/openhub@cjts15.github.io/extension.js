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

        const open = new PopupMenu.PopupMenuItem('Openhub');
        open.label.add_style_class_name('openhub-title');
        open.add_child(new St.Icon({
            icon_name: 'go-next-symbolic',
            style_class: 'popup-menu-icon',
            x_expand: true,
            x_align: Clutter.ActorAlign.END,
        }));
        open.connect('activate', () => showPage(''));
        this.menu.addMenuItem(open);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Batteries'));
        this._batteries = new PopupMenu.PopupMenuSection();
        this.menu.addMenuItem(this._batteries);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Bluetooth'));
        this._bluetooth = new PopupMenu.PopupMenuItem('');
        this._bluetooth.connect('activate', () => showPage('bluetooth'));
        this.menu.addMenuItem(this._bluetooth);

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem('Sound Output'));
        this._outputs = new PopupMenu.PopupMenuSection();
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
        const {description, origin} = device;
        const item = new PopupMenu.PopupMenuItem(origin ? `${description} – ${origin}` : description);
        item.connect('activate', () => {
            const output = this._mixer.lookup_output_id(id);
            if (output)
                this._mixer.change_output(output);
        });
        this._outputs.addMenuItem(item);
        this._outputItems.set(id, item);
    }

    _removeOutput(id) {
        this._outputItems.get(id)?.destroy();
        this._outputItems.delete(id);
    }

    _setActiveOutput(activeId) {
        for (const [id, item] of this._outputItems)
            item.setOrnament(id === activeId ? PopupMenu.Ornament.CHECK : PopupMenu.Ornament.NONE);
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
            const empty = new PopupMenu.PopupMenuItem('No devices with a battery', {reactive: false});
            this._batteries.addMenuItem(empty);
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
        this._bluetooth.label.text = bluetoothLabel(connected);
    }

    destroy() {
        this._cancellable.cancel();
        Gio.bus_unwatch_name(this._openhubWatch);
        this._mixer.disconnectObject(this);
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
 * A battery row: icon, then the name and percentage over a level bar the
 * width of the row. It opens Openhub's Power page, which has the details.
 *
 * @param {{name: string, icon: string, percentage: number, low: boolean}} row the device
 * @returns {PopupMenu.PopupBaseMenuItem} the item
 */
function batteryItem(row) {
    const item = new PopupMenu.PopupBaseMenuItem();
    item.connect('activate', () => showPage('power'));
    item.add_child(new St.Icon({icon_name: row.icon, style_class: 'popup-menu-icon'}));

    const text = new St.BoxLayout({x_expand: true});
    text.add_child(new St.Label({text: row.name, x_expand: true}));
    const percent = new St.Label({text: `${row.percentage}%`, style_class: 'openhub-percent'});
    text.add_child(percent);

    // A box packs the fill from the left; St.Bin would centre it.
    const bar = new St.BoxLayout({style_class: 'openhub-bar', x_expand: true});
    const fill = new St.Widget({style_class: 'openhub-bar-fill'});
    bar.add_child(fill);
    // The bar's width is only known once it's laid out.
    const fraction = Math.min(row.percentage, 100) / 100;
    bar.connect('notify::width', () => fill.set_width(Math.round(bar.width * fraction)));
    if (row.low) {
        percent.add_style_class_name('openhub-low');
        fill.add_style_class_name('openhub-low');
    }

    // A Clutter layout rather than St.BoxLayout's `vertical`, which GNOME 48 renamed.
    const column = new St.Widget({
        layout_manager: new Clutter.BoxLayout({orientation: Clutter.Orientation.VERTICAL, spacing: 4}),
        x_expand: true,
    });
    column.add_child(text);
    column.add_child(bar);
    item.add_child(column);
    return item;
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
