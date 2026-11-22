import { describe, expect, it } from 'vitest';
import { get, writable } from 'svelte/store';
import type { MediaStatus } from '../../../media/index';
import { createDeviceChoices } from './devices';

function setup(status: MediaStatus = 'joined') {
	const call = writable<{ status: MediaStatus }>({ status });
	return { call, devices: createDeviceChoices(call) };
}

describe('device choices', () => {
	it('start empty, meaning the browser default', () => {
		expect(get(setup().devices)).toEqual({ audioinput: '', videoinput: '' });
	});

	it('remember the last device picked for each kind', () => {
		const { devices } = setup();
		devices.remember('audioinput', 'mic-2');
		devices.remember('videoinput', 'cam-1');
		devices.remember('audioinput', 'mic-3');
		expect(get(devices)).toEqual({ audioinput: 'mic-3', videoinput: 'cam-1' });
	});

	it('survive status changes while the call is on (the panel closing does not matter here)', () => {
		const { call, devices } = setup();
		devices.remember('audioinput', 'mic-2');
		call.set({ status: 'connecting' });
		call.set({ status: 'joined' });
		expect(get(devices).audioinput).toBe('mic-2');
	});

	it('are forgotten when the call ends', () => {
		const { call, devices } = setup();
		devices.remember('audioinput', 'mic-2');
		devices.remember('videoinput', 'cam-1');
		call.set({ status: 'leaving' });
		expect(get(devices)).toEqual({ audioinput: '', videoinput: '' });
		devices.remember('audioinput', 'mic-9');
		call.set({ status: 'idle' });
		expect(get(devices)).toEqual({ audioinput: '', videoinput: '' });
	});
});
