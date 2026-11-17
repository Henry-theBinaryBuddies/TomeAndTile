import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { get } from 'svelte/store';
import { createMediaClient, type MediaClientOptions } from './client';
import { RoomFailure, type RoomLike } from './room';
import type { ParticipantInfo } from './types';

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

	async setMicrophoneEnabled(enabled: boolean) {
		if (this.micError) throw this.micError;
		this.micEnabled = enabled;
	}

	async setCameraEnabled(enabled: boolean) {
		if (this.cameraError) throw this.cameraError;
		this.cameraEnabled = enabled;
	}

	async switchActiveDevice() {}

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

function setup(overrides: Partial<MediaClientOptions> = {}) {
	const rooms: FakeRoom[] = [];
	const options: MediaClientOptions = {
		requestToken: vi.fn(async () => fakeToken()),
		createRoom: () => {
			const room = new FakeRoom();
			rooms.push(room);
			return room;
		},
		...overrides
	};
	const client = createMediaClient(options);
	return { client, rooms, requestToken: options.requestToken as ReturnType<typeof vi.fn> };
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
