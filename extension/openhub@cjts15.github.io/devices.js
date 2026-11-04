// SPDX-License-Identifier: GPL-3.0-or-later
//
// The menu's rules, kept free of GNOME Shell imports so `gjs -m` can test
// them on their own (see ../tests/devices.test.js).

/** A peripheral below this percentage is shown as low. */
export const LOW_BATTERY = 20;

// UPower's device types (UpDeviceKind) that are worth an icon of their own.
const ICONS = new Map([
    [5, 'input-mouse-symbolic'],
    [6, 'input-keyboard-symbolic'],
    [8, 'phone-symbolic'],
    [10, 'input-tablet-symbolic'],
    [12, 'input-gaming-symbolic'],
    [13, 'input-tablet-symbolic'],
    [14, 'input-touchpad-symbolic'],
    [17, 'audio-headset-symbolic'],
    [18, 'audio-speakers-symbolic'],
    [19, 'audio-headphones-symbolic'],
]);

// UPower's line-power type: chargers, never shown.
const LINE_POWER = 1;

/**
 * The batteries to list: peripherals only (not the laptop's own battery or
 * charger), each physical device once, lowest first.
 *
 * @param {Array<{type: number, model: string, serial: string,
 *   percentage: number, powerSupply: boolean, present: boolean}>} devices
 *   UPower's devices, as read from D-Bus
 * @returns {Array<{name: string, icon: string, percentage: number,
 *   low: boolean}>} the rows
 */
export function batteries(devices) {
    const seen = new Set();
    const rows = [];
    for (const device of devices) {
        if (device.powerSupply || !device.present || device.type === LINE_POWER)
            continue;
        // A Logitech mouse is listed twice, by its HID++ driver and by
        // BlueZ, with the same address as its serial.
        const key = device.serial.toLowerCase();
        if (key && seen.has(key))
            continue;
        seen.add(key);
        rows.push({
            name: device.model || 'Device',
            icon: ICONS.get(device.type) ?? 'battery-symbolic',
            percentage: Math.round(device.percentage),
            // Zero means UPower has no percentage for it, not an empty battery.
            low: device.percentage > 0 && device.percentage < LOW_BATTERY,
        });
    }
    return rows.sort((a, b) => a.percentage - b.percentage || a.name.localeCompare(b.name));
}

/**
 * The Bluetooth row's text.
 *
 * @param {number} connected the number of connected devices
 * @returns {string} the label
 */
export function bluetoothLabel(connected) {
    if (connected === 0)
        return 'No devices connected';
    return connected === 1 ? '1 device connected' : `${connected} devices connected`;
}
