// The microphone and camera the person last picked in the Video panel. The media module does
// not report the active device yet (issue #65), and the panel's body is unmounted when the
// panel is collapsed, the phone sheet is closed or the panel is moved, so the choice is kept
// here, outside the panel, for as long as the call lasts.
import { writable, type Readable } from 'svelte/store';
import type { DeviceKind, MediaStatus } from '../../../media/index';

export type DeviceChoices = Record<DeviceKind, string>;

const NONE: DeviceChoices = { audioinput: '', videoinput: '' };

export interface DeviceChoiceStore {
	subscribe: Readable<DeviceChoices>['subscribe'];
	/** Remember a device the call is now using. */
	remember(kind: DeviceKind, deviceId: string): void;
}

/** The choices are forgotten when the call ends (status goes back to idle or is leaving). */
export function createDeviceChoices(call: Readable<{ status: MediaStatus }>): DeviceChoiceStore {
	const store = writable<DeviceChoices>({ ...NONE });
	call.subscribe(({ status }) => {
		if (status === 'idle' || status === 'leaving') store.set({ ...NONE });
	});
	return {
		subscribe: store.subscribe,
		remember(kind, deviceId) {
			store.update((choices) => ({ ...choices, [kind]: deviceId }));
		}
	};
}
