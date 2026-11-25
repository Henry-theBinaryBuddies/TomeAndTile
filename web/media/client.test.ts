import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { createMediaClient, type MediaClientOptions } from './client';
import { RoomFailure, type RoomLike } from './room';
import type { DeviceKind, MediaDeviceOption, ParticipantInfo } from './types';

const SECRET_TOKEN = 'super-secret-join-token';

function fakeToken(overrides: Partial<Record<string, unknown>> = {}) {
	return {
		$typeName: 'tomeandtile.v1.MediaToken' as const,
		sfuUrl: 'wss://example.livekit.cloud',
		token: SECRET_TOKEN,
		room: 'campaign-1',
		expiresAtMs: BigInt(Date.now() + 60_000),
		canPublish: true,
		...overrides
	};
}

/** A `RoomLike` with no `livekit-client` import and no real SFU, per the issue's acceptance
 * tests. Lets a test fire `disconnect` and participant-change callbacks on demand. */
class FakeRoom implements RoomLike {
	connectCalls: Array<{ url: string; token: string }> = [];
	micEnabled = false;
	cameraEnabled = false;
	disconnected = false;
	connectError: unknown = null;
	micError: unknown = null;
	cameraError: unknown = null;
	switchDeviceError: unknown = null;
	switchDeviceCalls: Array<{ kind: DeviceKind; deviceId: string }> = [];
	private disconnectedCallbacks: Array<() => void> = [];
	private participantsChangedCallbacks: Array<() => void> = [];
	private participantList: ParticipantInfo[] = [
		{ id: 'local', name: 'Me', isLocal: true, micEnabled: false, cameraEnabled: false }
	];

	async connect(url: string, token: string) {
		this.connectCalls.push({ url, token });
		if (this.connectError) throw this.connectError;
	}

	async disconnect() {
		this.disconnected = true;
	}

	setMicrophoneDeviceId: string | undefined;
	setCameraDeviceId: string | undefined;

	async setMicrophoneEnabled(enabled: boolean, deviceId?: string) {
		if (this.micError) throw this.micError;
		this.micEnabled = enabled;
		this.setMicrophoneDeviceId = deviceId;
	}

	async setCameraEnabled(enabled: boolean, deviceId?: string) {
		if (this.cameraError) throw this.cameraError;
		this.cameraEnabled = enabled;
		this.setCameraDeviceId = deviceId;
	}

	async switchActiveDevice(kind: DeviceKind, deviceId: string) {
		this.switchDeviceCalls.push({ kind, deviceId });
		if (this.switchDeviceError) throw this.switchDeviceError;
	}

	participants() {
		return this.participantList;
	}

	setParticipants(list: ParticipantInfo[]) {
		this.participantList = list;
		for (const cb of this.participantsChangedCallbacks) cb();
	}

	attach() {
		return true;
	}

	detach() {}

	onDisconnected(callback: () => void) {
		this.disconnectedCallbacks.push(callback);
	}

	onParticipantsChanged(callback: () => void) {
		this.participantsChangedCallbacks.push(callback);
	}

	fireDisconnected() {
		this.disconnected = true;
		for (const cb of this.disconnectedCallbacks) cb();
	}
}

/** A fake device catalog + `devicechange` emitter, with no browser `navigator` involved. Tests
 * mutate `audioinput`/`videoinput` directly, then call `fireChange()` to simulate hardware
 * being plugged in or removed. */
class FakeDevices {
	audioinput: MediaDeviceOption[] = [
		{ deviceId: 'mic-1', label: 'Mic 1' },
		{ deviceId: 'mic-2', label: 'Mic 2' }
	];
	videoinput: MediaDeviceOption[] = [{ deviceId: 'cam-1', label: 'Cam 1' }];
	private listeners: Array<() => void> = [];
	listCalls: DeviceKind[] = [];

	list = async (kind: DeviceKind): Promise<MediaDeviceOption[]> => {
		this.listCalls.push(kind);
		return [...this[kind]];
	};

	onChange = (callback: () => void): (() => void) => {
		this.listeners.push(callback);
		return () => {
			this.listeners = this.listeners.filter((l) => l !== callback);
		};
	};

	fireChange() {
		for (const listener of this.listeners) listener();
	}
}

function setup(overrides: Partial<MediaClientOptions> = {}, devices = new FakeDevices()) {
	const rooms: FakeRoom[] = [];
	const options: MediaClientOptions = {
		requestToken: vi.fn(async () => fakeToken()),
		createRoom: () => {
			const room = new FakeRoom();
			rooms.push(room);
			return room;
		},
		listDevices: devices.list,
		onDeviceChange: devices.onChange,
		...overrides
	};
	const client = createMediaClient(options);
	return {
		client,
		rooms,
		devices,
		requestToken: options.requestToken as ReturnType<typeof vi.fn>
	};
}

beforeEach(() => {
	vi.useFakeTimers();
});

afterEach(() => {
	vi.useRealTimers();
});

describe('join', () => {
	it('requests a token and connects to sfu_url with it', async () => {
		const { client, rooms, requestToken } = setup();
		await client.join();

		expect(requestToken).toHaveBeenCalledTimes(1);
		expect(rooms[0].connectCalls).toEqual([
			{ url: 'wss://example.livekit.cloud', token: SECRET_TOKEN }
		]);
		expect(get(client.state).status).toBe('joined');
	});

	it('opens the microphone only after connecting, when can_publish is true', async () => {
		const { client, rooms } = setup();
		await client.join();

		expect(rooms[0].micEnabled).toBe(true);
		expect(get(client.state).micEnabled).toBe(true);
	});

	it('never opens the camera or microphone for a token with can_publish false', async () => {
		const { client, rooms } = setup({
			requestToken: async () => fakeToken({ canPublish: false })
		});
		await client.join();

		expect(rooms[0].micEnabled).toBe(false);
		expect(rooms[0].cameraEnabled).toBe(false);
		expect(get(client.state).canPublish).toBe(false);
	});

	it('does not request a token or connect at construction time, only on join', () => {
		const { requestToken, rooms } = setup();
		expect(requestToken).not.toHaveBeenCalled();
		expect(rooms).toHaveLength(0);
	});
});

describe('reconnect on a dropped connection', () => {
	it('asks for a new token and rejoins after a short delay when the connection drops', async () => {
		const { client, rooms, requestToken } = setup();
		await client.join();

		rooms[0].fireDisconnected();
		// Checked synchronously: the automatic rejoin clears `failure` again once it starts, so
		// this is the only window where the drop is visible through the subscription.
		expect(get(client.state).failure?.kind).toBe('connection_lost');
		expect(requestToken).toHaveBeenCalledTimes(1); // not yet; the rejoin is delayed

		await vi.advanceTimersByTimeAsync(1_000); // base reconnect delay
		expect(requestToken).toHaveBeenCalledTimes(2);
		expect(rooms[1]).toBeDefined();
		expect(get(client.state).status).toBe('joined');
	});

	it('grows the delay on repeated drops, and resets it only once the connection stays up', async () => {
		const { client, rooms, requestToken } = setup();
		await client.join();

		rooms[0].fireDisconnected();
		await vi.advanceTimersByTimeAsync(1_000); // 1st delay: base
		expect(requestToken).toHaveBeenCalledTimes(2);

		// Dropped again right away, before the join above could count as stable: the delay
		// keeps growing instead of resetting. This is the "accepts and drops at once" case the
		// backoff exists for — resetting on every `connect()` resolving, no matter how briefly
		// the call stays up, would never actually slow anything down.
		rooms[1].fireDisconnected();
		await vi.advanceTimersByTimeAsync(1_000); // too soon: delay has grown to 2s
		expect(requestToken).toHaveBeenCalledTimes(2);
		await vi.advanceTimersByTimeAsync(1_000); // now at 2s total
		expect(requestToken).toHaveBeenCalledTimes(3);

		// This time, let the join stay up long enough to count as stable before it drops again.
		await vi.advanceTimersByTimeAsync(5_000); // STABLE_CONNECTION_MS
		rooms[2].fireDisconnected();
		await vi.advanceTimersByTimeAsync(1_000); // back to the base delay
		expect(requestToken).toHaveBeenCalledTimes(4);
	});

	it('cancels a pending reconnect when left before it fires', async () => {
		const { client, rooms, requestToken } = setup();
		await client.join();

		rooms[0].fireDisconnected();
		await client.leave();

		await vi.advanceTimersByTimeAsync(60_000);
		expect(requestToken).toHaveBeenCalledTimes(1); // the pending reconnect never fired
		expect(get(client.state).status).toBe('idle');
		expect(rooms).toHaveLength(1);
	});
});

describe('leave', () => {
	it('disconnects and returns to a clean idle state', async () => {
		const { client, rooms } = setup();
		await client.join();
		await client.leave();

		expect(rooms[0].disconnected).toBe(true);
		const state = get(client.state);
		expect(state.status).toBe('idle');
		expect(state.micEnabled).toBe(false);
		expect(state.cameraEnabled).toBe(false);
		expect(state.participants).toEqual([]);
	});

});

describe('mute, unmute, camera', () => {
	it('mute and unmute toggle the microphone without leaving', async () => {
		const { client, rooms } = setup();
		await client.join();

		client.mute();
		await vi.advanceTimersByTimeAsync(0);
		expect(rooms[0].micEnabled).toBe(false);
		expect(get(client.state).micEnabled).toBe(false);

		client.unmute();
		await vi.advanceTimersByTimeAsync(0);
		expect(rooms[0].micEnabled).toBe(true);
		expect(get(client.state).micEnabled).toBe(true);
	});

	it('mute does nothing for a spectator (can_publish false)', async () => {
		const { client, rooms } = setup({ requestToken: async () => fakeToken({ canPublish: false }) });
		await client.join();

		client.mute();
		await vi.advanceTimersByTimeAsync(0);
		expect(rooms[0].micEnabled).toBe(false);
	});

	it('enableCamera and disableCamera toggle the camera', async () => {
		const { client, rooms } = setup();
		await client.join();

		await client.enableCamera();
		expect(rooms[0].cameraEnabled).toBe(true);
		expect(get(client.state).cameraEnabled).toBe(true);

		client.disableCamera();
		await vi.advanceTimersByTimeAsync(0);
		expect(rooms[0].cameraEnabled).toBe(false);
	});
});

describe('failures leave a clean state', () => {
	it('a refused token leaves status idle with a token_refused failure', async () => {
		const { client } = setup({
			requestToken: async () => {
				throw new Error('server said no');
			}
		});
		await client.join();

		const state = get(client.state);
		expect(state.status).toBe('idle');
		expect(state.failure).toEqual({ kind: 'token_refused', message: 'server said no' });
	});

	it('a connect failure classified as token_refused by the room leaves a clean state', async () => {
		const requestToken = vi.fn(async () => fakeToken());
		const room = new FakeRoom();
		room.connectError = new RoomFailure('token_refused', 'bad signature');
		const client = createMediaClient({ requestToken, createRoom: () => room });
		await client.join();

		const state = get(client.state);
		expect(state.status).toBe('idle');
		expect(state.failure).toEqual({ kind: 'token_refused', message: 'bad signature' });
	});

	it('permission denied on the microphone is reported without leaving the call', async () => {
		const requestToken = vi.fn(async () => fakeToken());
		const room = new FakeRoom();
		room.micError = new RoomFailure('permission_denied', 'denied by the user');
		const client = createMediaClient({ requestToken, createRoom: () => room });
		await client.join();

		const state = get(client.state);
		expect(state.status).toBe('joined'); // still in the room; just no microphone
		expect(state.micEnabled).toBe(false);
		expect(state.failure).toEqual({ kind: 'permission_denied', message: 'denied by the user' });
	});

	it('no_device on enableCamera is reported', async () => {
		const requestToken = vi.fn(async () => fakeToken());
		const room = new FakeRoom();
		const client = createMediaClient({ requestToken, createRoom: () => room });
		await client.join();

		room.cameraError = new RoomFailure('no_device', 'no camera found');
		await client.enableCamera();

		expect(get(client.state).failure).toEqual({ kind: 'no_device', message: 'no camera found' });
		expect(get(client.state).cameraEnabled).toBe(false);
	});
});

describe('participants', () => {
	it('listParticipants reflects onParticipantsChanged', async () => {
		const { client, rooms } = setup();
		await client.join();

		rooms[0].setParticipants([
			{ id: 'local', name: 'Me', isLocal: true, micEnabled: true, cameraEnabled: false },
			{ id: 'gm', name: 'The GM', isLocal: false, micEnabled: false, cameraEnabled: true }
		]);

		expect(client.listParticipants()).toHaveLength(2);
		expect(client.listParticipants()[1].name).toBe('The GM');
	});
});

describe('device handling', () => {
	it('lists both kinds at startup without asking permission, reaching state', async () => {
		const { client, devices } = setup();
		await vi.advanceTimersByTimeAsync(0);

		expect(devices.listCalls).toEqual(expect.arrayContaining(['audioinput', 'videoinput']));
		const state = get(client.state);
		expect(state.devices.audioinput).toEqual([
			{ deviceId: 'mic-1', label: 'Mic 1' },
			{ deviceId: 'mic-2', label: 'Mic 2' }
		]);
		expect(state.devices.videoinput).toEqual([{ deviceId: 'cam-1', label: 'Cam 1' }]);
	});

	it('re-lists on a devicechange event; the panel never has to poll', async () => {
		const { client, devices } = setup();
		await vi.advanceTimersByTimeAsync(0);
		devices.listCalls = [];

		devices.audioinput = [
			{ deviceId: 'mic-1', label: 'Mic 1' },
			{ deviceId: 'mic-2', label: 'Mic 2' },
			{ deviceId: 'mic-3', label: 'Mic 3' }
		];
		devices.fireChange();
		await vi.advanceTimersByTimeAsync(0);

		expect(devices.listCalls).toContain('audioinput');
		expect(get(client.state).devices.audioinput).toHaveLength(3);
	});

	it('records the selected device in state immediately, even before joining', async () => {
		const { client } = setup();
		await client.selectDevice('audioinput', 'mic-2');

		expect(get(client.state).selectedDeviceId.audioinput).toBe('mic-2');
	});

	it('applies a selection made before joining by capturing that device from the start', async () => {
		const { client, rooms } = setup();
		await client.selectDevice('audioinput', 'mic-2');
		await client.join();

		// Passed straight to setMicrophoneEnabled, not switched to afterward: the wrong
		// microphone must never open even briefly.
		expect(rooms[0].setMicrophoneDeviceId).toBe('mic-2');
		expect(rooms[0].switchDeviceCalls).toEqual([]);
	});

	it('still applies the remembered selection after a reconnect', async () => {
		const { client, rooms } = setup();
		await client.selectDevice('audioinput', 'mic-2');
		await client.join();

		rooms[0].fireDisconnected();
		await vi.advanceTimersByTimeAsync(1_000);

		expect(rooms[1].setMicrophoneDeviceId).toBe('mic-2');
	});

	it('with no explicit selection, opens the default device rather than naming one in state', async () => {
		const { client, rooms } = setup();
		await vi.advanceTimersByTimeAsync(0); // let the startup listing resolve

		expect(get(client.state).selectedDeviceId).toEqual({ audioinput: null, videoinput: null });

		await client.join();

		// No deviceId constraint: the system default opens, not whatever happened to list first.
		expect(rooms[0].setMicrophoneDeviceId).toBeUndefined();
		expect(get(client.state).selectedDeviceId.audioinput).toBeNull();
	});

	it('selecting a device never opens one for a spectator', async () => {
		const { client, rooms } = setup({ requestToken: async () => fakeToken({ canPublish: false }) });
		await client.join();
		await client.selectDevice('audioinput', 'mic-2');

		expect(rooms[0].switchDeviceCalls).toEqual([]);
	});

	it('falls back to another device when the selected one disappears, and switches the live track', async () => {
		const { client, rooms, devices } = setup();
		await client.selectDevice('audioinput', 'mic-1');
		await client.join();

		devices.audioinput = [{ deviceId: 'mic-2', label: 'Mic 2' }];
		devices.fireChange();
		await vi.advanceTimersByTimeAsync(0);

		expect(get(client.state).selectedDeviceId.audioinput).toBe('mic-2');
		expect(rooms[0].switchDeviceCalls).toContainEqual({ kind: 'audioinput', deviceId: 'mic-2' });
	});

	it('reports no_device when the selected device disappears and none are left', async () => {
		const { client, devices } = setup();
		await client.selectDevice('audioinput', 'mic-1');
		await client.join();

		devices.audioinput = [];
		devices.fireChange();
		await vi.advanceTimersByTimeAsync(0);

		expect(get(client.state).failure).toEqual({ kind: 'no_device', message: 'No microphone is available.' });
	});

	it('a failed selectDevice lands in state.failure and the call stays up', async () => {
		const { client, rooms } = setup();
		await client.join();
		rooms[0].switchDeviceError = new RoomFailure('no_device', 'that microphone is gone');

		await client.selectDevice('audioinput', 'mic-2');

		const state = get(client.state);
		expect(state.failure).toEqual({ kind: 'no_device', message: 'that microphone is gone' });
		expect(state.status).toBe('joined');
	});

	it('re-lists devices once permission is granted, to pick up real labels', async () => {
		const { client, devices } = setup();
		devices.listCalls = [];

		await client.join(); // can_publish defaults true, so this enables the microphone

		expect(devices.listCalls).toContain('audioinput');
	});
});

describe('the token never leaks', () => {
	it('never appears in state, console output, or an attach/detach call', async () => {
		const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
		const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
		const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

		const { client } = setup();
		await client.join();
		client.mute();
		await client.leave();

		const state = get(client.state);
		expect(JSON.stringify(state)).not.toContain(SECRET_TOKEN);

		for (const spy of [logSpy, warnSpy, errorSpy]) {
			for (const call of spy.mock.calls) {
				expect(JSON.stringify(call)).not.toContain(SECRET_TOKEN);
			}
			spy.mockRestore();
		}
	});
});
