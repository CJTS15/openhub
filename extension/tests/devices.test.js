// SPDX-License-Identifier: GPL-3.0-or-later
//
// Run with: gjs -m extension/tests/devices.test.js

import {batteries} from '../openhub@cjts15.github.io/devices.js';

/**
 * @param {unknown} actual the value under test
 * @param {unknown} expected what it should be
 * @param {string} what a name for the failure message
 */
function equal(actual, expected, what) {
    const a = JSON.stringify(actual), e = JSON.stringify(expected);
    if (a !== e)
        throw new Error(`${what}: expected ${e}, got ${a}`);
}

const device = (type, model, serial, percentage, extra = {}) =>
    ({type, model, serial, percentage, powerSupply: false, present: true, ...extra});

// As UPower lists CJ's laptop: its own battery and charger, the MX Master 3S
// twice (HID++ and BlueZ), and the AL68 over Bluetooth.
const rows = batteries([
    device(2, 'L22M3PG4', '1234', 79, {powerSupply: true}),
    device(1, '', '', 0, {powerSupply: true}),
    device(5, 'MX Master 3S', 'aa:bb:cc:00:00:01', 12),
    device(5, 'MX Master 3S', 'AA:BB:CC:00:00:01', 12),
    device(6, 'YUNZII AL68 BT1', 'AA:BB:CC:00:00:02', 64),
    device(19, 'soundcore A20i', 'aa:bb:cc:00:00:03', 0),
    device(17, 'Gone headset', 'aa:bb:cc:00:00:04', 50, {present: false}),
]);
equal(rows, [
    {name: 'soundcore A20i', icon: 'audio-headphones-symbolic', percentage: 0, low: false},
    {name: 'MX Master 3S', icon: 'input-mouse-symbolic', percentage: 12, low: true},
    {name: 'YUNZII AL68 BT1', icon: 'input-keyboard-symbolic', percentage: 64, low: false},
], 'batteries');

equal(batteries([device(99, '', '', 40)]),
    [{name: 'Device', icon: 'battery-symbolic', percentage: 40, low: false}], 'unknown type');


print('devices.test.js: ok');
