// The video/voice client module (ADR 0007, web/media/CLAUDE.md). Owns the LiveKit room for one
// call, exposes only the small interface below, and never changes state outside of it.
//
// The token is kept in the closure only: never assigned to a module-level variable, never
// logged, never read back out of `MediaClientState`. See `client.test.ts` for the test that
// asserts this.
import { readable, type Readable } from 'svelte/store';
import type { MediaToken } from '@tomeandtile/contracts';
import { listLocalDevices, wrapLiveKitRoom, RoomFailure, type RoomLike } from './room';
import type {
	DeviceKind,
	MediaClientState,
	MediaDeviceOption,
	MediaFailure,
	ParticipantInfo,
	TrackKind
} from './types';

export type { DeviceKind, MediaClientState, MediaDeviceOption, MediaFailure, ParticipantInfo, TrackKind };

const DEFAULT_RENEW_BEFORE_MS = 10_000;

export interface MediaClientOptions {
	/** Returns the next join token. Backed by the game socket client's `requestMediaToken()`. */
	requestToken: () => Promise<MediaToken>;
	/** Creates the room for one join. Defaults to a real LiveKit room; tests supply a fake. */
	createRoom?: () => RoomLike;
	/** Wall clock, for scheduling the pre-expiry rejoin. Defaults to `Date.now`. */
	now?: () => number;
	/** How long before `expires_at_ms` to ask for a new token and rejoin. Defaults to 10s. */
	renewBeforeMs?: number;
}

export interface MediaClient {
	readonly state: Readable<MediaClientState>;
	join(): Promise<void>;
	leave(): Promise<void>;
	mute(): void;
	unmute(): void;
	enableCamera(): Promise<void>;
	disableCamera(): void;
	listDevices(kind: DeviceKind): Promise<MediaDeviceOption[]>;
	selectDevice(kind: DeviceKind, deviceId: string): Promise<void>;
	listParticipants(): ParticipantInfo[];
	attachTrack(participantId: string, kind: TrackKind, element: HTMLMediaElement): void;
	detachTrack(participantId: string, kind: TrackKind, element: HTMLMediaElement): void;
}

function initialState(): MediaClientState {
	return {
		status: 'idle',
		canPublish: false,
		micEnabled: false,
		cameraEnabled: false,
		participants: [],
		failure: null
	};
}

function toFailure(error: unknown): MediaFailure {
	if (error instanceof RoomFailure && error.kind !== 'other') {
		return { kind: error.kind, message: error.message };
	}
	return { kind: 'no_device', message: error instanceof Error ? error.message : String(error) };
}

export function createMediaClient(options: MediaClientOptions): MediaClient {
	const {
		requestToken,
		createRoom = () => wrapLiveKitRoom(),
		now = () => Date.now(),
		renewBeforeMs = DEFAULT_RENEW_BEFORE_MS
	} = options;

	let state = initialState();
	let room: RoomLike | null = null;
	let renewTimer: ReturnType<typeof setTimeout> | null = null;
	// Bumped on every join/leave. A callback from an earlier join compares against this before
	// touching `state` or `room`, so a stale rejoin or disconnect handler can never clobber a
	// newer one.
	let generation = 0;
	const subscribers = new Set<(state: MediaClientState) => void>();

	function setState(patch: Partial<MediaClientState>) {
		state = { ...state, ...patch };
		for (const subscriber of subscribers) subscriber(state);
	}

	function clearRenewTimer() {
		if (renewTimer !== null) {
			clearTimeout(renewTimer);
			renewTimer = null;
		}
	}

	function scheduleRenew(expiresAtMs: number, myGeneration: number) {
		clearRenewTimer();
		const delay = Math.max(0, expiresAtMs - now() - renewBeforeMs);
		renewTimer = setTimeout(() => {
			if (myGeneration === generation) void rejoin(myGeneration);
		}, delay);
	}

	async function teardown() {
		clearRenewTimer();
		const current = room;
		room = null;
		if (current) {
			try {
				await current.disconnect();
			} catch {
				// Already gone (for example after a connection drop); nothing left to tear down.
			}
		}
	}

	async function rejoin(myGeneration: number) {
		await teardown();
		if (myGeneration !== generation) return; // a newer join/leave took over while we waited
		await connect(myGeneration);
	}

	async function connect(myGeneration: number) {
		setState({ status: 'connecting', failure: null });

		let token: MediaToken;
		try {
			token = await requestToken();
		} catch (error) {
			if (myGeneration !== generation) return;
			setState({ status: 'idle', failure: { kind: 'token_refused', message: describeError(error) } });
			return;
		}
		if (myGeneration !== generation) return;

		const nextRoom = createRoom();
		try {
			await nextRoom.connect(token.sfuUrl, token.token);
		} catch (error) {
			if (myGeneration !== generation) return;
			setState({ status: 'idle', failure: toFailure(error) });
			return;
		}
		if (myGeneration !== generation) {
			await nextRoom.disconnect().catch(() => {});
			return;
		}

		room = nextRoom;
		nextRoom.onDisconnected(() => {
			if (myGeneration !== generation || room !== nextRoom) return;
			room = null;
			setState({
				status: 'idle',
				micEnabled: false,
				cameraEnabled: false,
				failure: { kind: 'connection_lost', message: 'Disconnected from the call.' }
			});
			void rejoin(myGeneration);
		});
		nextRoom.onParticipantsChanged(() => {
			if (myGeneration !== generation || room !== nextRoom) return;
			setState({ participants: nextRoom.participants() });
		});

		setState({ status: 'joined', canPublish: token.canPublish, participants: nextRoom.participants() });
		scheduleRenew(Number(token.expiresAtMs), myGeneration);

		if (token.canPublish) {
			try {
				await nextRoom.setMicrophoneEnabled(true);
				if (myGeneration !== generation) return;
				setState({ micEnabled: true });
			} catch (error) {
				if (myGeneration !== generation) return;
				setState({ failure: toFailure(error) });
			}
		}
	}

	return {
		state: readable(state, (set) => {
			const subscriber = (next: MediaClientState) => set(next);
			subscribers.add(subscriber);
			set(state);
			return () => subscribers.delete(subscriber);
		}),

		async join() {
			if (state.status === 'connecting' || state.status === 'joined') return;
			const myGeneration = ++generation;
			await connect(myGeneration);
		},

		async leave() {
			generation++; // invalidate any in-flight connect/rejoin/event handler
			setState({ status: 'leaving' });
			await teardown();
			setState(initialState());
		},

		mute() {
			if (!room || !state.canPublish) return;
			room
				.setMicrophoneEnabled(false)
				.then(() => setState({ micEnabled: false }))
				.catch((error) => setState({ failure: toFailure(error) }));
		},

		unmute() {
			if (!room || !state.canPublish) return;
			room
				.setMicrophoneEnabled(true)
				.then(() => setState({ micEnabled: true }))
				.catch((error) => setState({ failure: toFailure(error) }));
		},

		async enableCamera() {
			if (!room || !state.canPublish) return;
			try {
				await room.setCameraEnabled(true);
				setState({ cameraEnabled: true });
			} catch (error) {
				setState({ failure: toFailure(error) });
			}
		},

		disableCamera() {
			if (!room) return;
			room
				.setCameraEnabled(false)
				.then(() => setState({ cameraEnabled: false }))
				.catch((error) => setState({ failure: toFailure(error) }));
		},

		listDevices(kind) {
			return listLocalDevices(kind);
		},

		async selectDevice(kind, deviceId) {
			if (!room) return;
			await room.switchActiveDevice(kind, deviceId);
		},

		listParticipants() {
			return state.participants;
		},

		attachTrack(participantId, kind, element) {
			room?.attach(participantId, kind, element);
		},

		detachTrack(participantId, kind, element) {
			room?.detach(participantId, kind, element);
		}
	};
}

function describeError(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}
