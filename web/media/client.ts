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
	DeviceLists,
	MediaClientState,
	MediaDeviceOption,
	MediaFailure,
	ParticipantInfo,
	SelectedDeviceIds,
	TrackAvailability,
	TrackKind
} from './types';

export type {
	DeviceKind,
	DeviceLists,
	MediaClientState,
	MediaDeviceOption,
	MediaFailure,
	ParticipantInfo,
	SelectedDeviceIds,
	TrackAvailability,
	TrackKind
};

// A token's expiry only matters for the initial connection: LiveKit's client library refreshes
// its own token for a network-drop reconnect, so there is no pre-expiry renewal here (see
// https://docs.livekit.io/home/get-started/authentication/#access-tokens-and-grants). What this
// module does handle is a dropped connection: it rejoins (asking the server for a new token)
// with a delay that grows on repeated failures, so a server that accepts and immediately drops
// the connection cannot spin in a tight loop.
const BASE_RECONNECT_DELAY_MS = 1_000;
const MAX_RECONNECT_DELAY_MS = 16_000;
// A join only counts as "successful" for backoff purposes once the connection has stayed up
// this long. Resetting the backoff the instant `connect()` resolves would let a server that
// accepts and immediately drops the connection reset to the base delay every cycle, which is
// exactly the tight loop the backoff exists to prevent.
const STABLE_CONNECTION_MS = 5_000;

export interface MediaClientOptions {
	/** Returns the next join token. Backed by the game socket client's `requestMediaToken()`. */
	requestToken: () => Promise<MediaToken>;
	/** Creates the room for one join. Defaults to a real LiveKit room; tests supply a fake. */
	createRoom?: () => RoomLike;
	/** Lists local devices of one kind. Defaults to `listLocalDevices` (never asks for
	 * permission); tests supply a fake catalog. */
	listDevices?: (kind: DeviceKind) => Promise<MediaDeviceOption[]>;
	/** Subscribes to the browser's device-plugged-in/removed event; returns an unsubscribe
	 * function. Defaults to `navigator.mediaDevices`'s `devicechange` event; tests fire it by
	 * hand instead of simulating real hardware. */
	onDeviceChange?: (callback: () => void) => () => void;
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
	/** Returns whether a track was actually there to attach. */
	attachTrack(participantId: string, kind: TrackKind, element: HTMLMediaElement): boolean;
	detachTrack(participantId: string, kind: TrackKind, element: HTMLMediaElement): void;
	/** Resumes playback after the browser blocked it (`state.audioPlaybackBlocked`). Call this
	 * from a user gesture, such as a click handler on a "tap to enable sound" prompt. */
	startAudioPlayback(): Promise<void>;
}

function emptyDeviceLists(): DeviceLists {
	return { audioinput: [], videoinput: [] };
}

function noSelectedDevices(): SelectedDeviceIds {
	return { audioinput: null, videoinput: null };
}

function initialState(): MediaClientState {
	return {
		status: 'idle',
		canPublish: false,
		micEnabled: false,
		cameraEnabled: false,
		participants: [],
		failure: null,
		devices: emptyDeviceLists(),
		selectedDeviceId: noSelectedDevices(),
		audioPlaybackBlocked: false
	};
}

function defaultOnDeviceChange(callback: () => void): () => void {
	// No-op outside a real browser (SSR, or a test that constructs the client directly without
	// supplying this option) rather than throwing: device-change notification is best-effort,
	// the same spirit as `refreshDevices` swallowing a listing failure.
	if (typeof navigator === 'undefined' || !navigator.mediaDevices) return () => {};
	navigator.mediaDevices.addEventListener('devicechange', callback);
	return () => navigator.mediaDevices.removeEventListener('devicechange', callback);
}

function deviceLabel(kind: DeviceKind): string {
	return kind === 'audioinput' ? 'microphone' : 'camera';
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
		listDevices: fetchDeviceList = listLocalDevices,
		onDeviceChange = defaultOnDeviceChange
	} = options;

	let state = initialState();
	let room: RoomLike | null = null;
	// The user's chosen device per kind, remembered across leave/join and reconnects. Applied
	// to the room right after that kind's track is first enabled (see `applyRememberedDevice`).
	const selectedDeviceId: SelectedDeviceIds = noSelectedDevices();
	// Bumped every time something sets `selectedDeviceId[kind]` (an explicit `selectDevice`, or
	// `refreshDevices`'s own fallback). A `refreshDevices` call that started before such a change
	// checks this after its (asynchronous) device listing resolves, and skips touching the
	// selection if it no longer matches — otherwise a device list fetched before a fresh
	// `selectDevice` call could resolve after it and silently overwrite the user's new choice.
	const selectionVersion: Record<DeviceKind, number> = { audioinput: 0, videoinput: 0 };

	function setSelectedDevice(kind: DeviceKind, deviceId: string | null) {
		selectionVersion[kind]++;
		selectedDeviceId[kind] = deviceId;
	}
	let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
	let stabilityTimer: ReturnType<typeof setTimeout> | null = null;
	// Grows on every dropped-connection rejoin, reset to 0 once a join stays up for
	// STABLE_CONNECTION_MS.
	let reconnectAttempt = 0;
	// Bumped on every join/leave. A callback from an earlier join compares against this before
	// touching `state` or `room`, so a stale rejoin or disconnect handler can never clobber a
	// newer one.
	let generation = 0;
	const subscribers = new Set<(state: MediaClientState) => void>();

	function setState(patch: Partial<MediaClientState>) {
		state = { ...state, ...patch };
		for (const subscriber of subscribers) subscriber(state);
	}

	// Re-lists one kind of device and, only when there is an explicit remembered selection,
	// reconciles it against the fresh list: if that device is gone, falls back to whatever is
	// first in the new list (or reports no_device if nothing of that kind is left), and moves
	// the live track there when one is active. With no explicit selection, `null` is left as it
	// is ("use the browser's own default") rather than naming a specific device: the first
	// device in a list is not necessarily the system default (Firefox lists no explicit default
	// entry at all), so writing it into `selectedDeviceId` would make the state claim a choice
	// nobody made and silently stop the no-device/fallback handling above from ever running for
	// a device that was never actually selected. Never throws — listing devices is best-effort
	// and must not break a call.
	async function refreshDevices(kind: DeviceKind) {
		const versionAtStart = selectionVersion[kind];
		let list: MediaDeviceOption[];
		try {
			list = await fetchDeviceList(kind);
		} catch {
			return;
		}

		if (selectionVersion[kind] !== versionAtStart) {
			// Something else (an explicit selectDevice, or another refreshDevices call) already
			// changed the selection while this listing was in flight. That change is newer and
			// more deliberate than whatever this call would conclude from a now-stale list, so
			// only update what is shown, never the selection itself.
			setState({ devices: { ...state.devices, [kind]: list } });
			return;
		}

		const hadSelection = selectedDeviceId[kind];
		if (hadSelection === null || list.some((device) => device.deviceId === hadSelection)) {
			setState({ devices: { ...state.devices, [kind]: list } });
			return;
		}

		// The explicitly selected device is gone: fall back to whatever is first in the new
		// list, or report no_device if nothing of that kind is left.
		const nextSelection = list[0]?.deviceId ?? null;
		setSelectedDevice(kind, nextSelection);
		setState({
			devices: { ...state.devices, [kind]: list },
			selectedDeviceId: { ...state.selectedDeviceId, [kind]: nextSelection }
		});

		if (nextSelection === null) {
			setState({ failure: { kind: 'no_device', message: `No ${deviceLabel(kind)} is available.` } });
			return;
		}
		if (room && state.canPublish) {
			try {
				await room.switchActiveDevice(kind, nextSelection);
			} catch (error) {
				setState({ failure: toFailure(error) });
			}
		}
	}

	onDeviceChange(() => {
		void refreshDevices('audioinput');
		void refreshDevices('videoinput');
	});
	void refreshDevices('audioinput');
	void refreshDevices('videoinput');

	function clearReconnectTimer() {
		if (reconnectTimer !== null) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
	}

	function clearStabilityTimer() {
		if (stabilityTimer !== null) {
			clearTimeout(stabilityTimer);
			stabilityTimer = null;
		}
	}

	function scheduleReconnect(myGeneration: number) {
		clearReconnectTimer();
		const delay = Math.min(BASE_RECONNECT_DELAY_MS * 2 ** reconnectAttempt, MAX_RECONNECT_DELAY_MS);
		reconnectAttempt++;
		reconnectTimer = setTimeout(() => {
			reconnectTimer = null;
			if (myGeneration === generation) void rejoin(myGeneration);
		}, delay);
	}

	async function teardown() {
		clearReconnectTimer();
		clearStabilityTimer();
		const current = room;
		room = null;
		if (current) {
			// Remove our listeners before disconnecting, so a disconnect-triggered event firing
			// on the way out can never touch state for a room we've already moved on from.
			current.dispose();
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
			scheduleReconnect(myGeneration);
		});
		nextRoom.onParticipantsChanged(() => {
			if (myGeneration !== generation || room !== nextRoom) return;
			setState({ participants: nextRoom.participants() });
		});
		nextRoom.onAudioPlaybackStatusChanged((blocked) => {
			if (myGeneration !== generation || room !== nextRoom) return;
			setState({ audioPlaybackBlocked: blocked });
		});

		clearStabilityTimer();
		stabilityTimer = setTimeout(() => {
			stabilityTimer = null;
			if (myGeneration === generation) reconnectAttempt = 0;
		}, STABLE_CONNECTION_MS);
		setState({
			status: 'joined',
			canPublish: token.canPublish,
			participants: nextRoom.participants(),
			audioPlaybackBlocked: nextRoom.isAudioPlaybackBlocked()
		});

		if (token.canPublish) {
			try {
				// The remembered device, if any, is captured from the start (instead of the
				// system default and then switched), so the wrong microphone never opens even
				// briefly.
				await nextRoom.setMicrophoneEnabled(true, selectedDeviceId.audioinput ?? undefined);
				if (myGeneration !== generation) return;
				setState({ micEnabled: true });
				void refreshDevices('audioinput');
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
			// Reset everything about the call, but not the device catalog or the user's chosen
			// device: those are standing preferences, not per-call state, and should still show
			// correctly while idle and still apply to the next join.
			setState({
				status: 'idle',
				canPublish: false,
				micEnabled: false,
				cameraEnabled: false,
				participants: [],
				failure: null
			});
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
				// Same reasoning as the microphone in connect(): capture the remembered device
				// from the start, never switch to it after the wrong one is already live.
				await room.setCameraEnabled(true, selectedDeviceId.videoinput ?? undefined);
				setState({ cameraEnabled: true });
				void refreshDevices('videoinput');
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
			return fetchDeviceList(kind);
		},

		async selectDevice(kind, deviceId) {
			setSelectedDevice(kind, deviceId);
			setState({ selectedDeviceId: { ...state.selectedDeviceId, [kind]: deviceId } });
			if (!room || !state.canPublish) return; // remembered for the next enable; nothing live to switch yet
			try {
				await room.switchActiveDevice(kind, deviceId);
			} catch (error) {
				setState({ failure: toFailure(error) });
			}
		},

		listParticipants() {
			return state.participants;
		},

		attachTrack(participantId, kind, element) {
			return room?.attach(participantId, kind, element) ?? false;
		},

		detachTrack(participantId, kind, element) {
			room?.detach(participantId, kind, element);
		},

		async startAudioPlayback() {
			if (!room) return;
			try {
				await room.startAudioPlayback();
			} catch (error) {
				setState({ failure: toFailure(error) });
				return;
			}
			setState({ audioPlaybackBlocked: room.isAudioPlaybackBlocked() });
		}
	};
}

function describeError(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}
