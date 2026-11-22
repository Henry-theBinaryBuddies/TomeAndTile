// The only file in `web/media/` that imports `livekit-client` (ADR 0007, web/media/CLAUDE.md).
// `client.ts` talks only to the `RoomLike` interface below, so a test can supply a fake room
// with no LiveKit import and no real SFU, and the exported `MediaClient` interface in
// `client.ts` never carries a LiveKit type.
import {
	MediaDeviceFailure,
	Room,
	RoomEvent,
	Track,
	type LocalParticipant,
	type Participant
} from 'livekit-client';
import type { DeviceKind, MediaDeviceOption, MediaFailureKind, ParticipantInfo, TrackKind } from './types';

/** A failure classified well enough for `client.ts` to turn into a `MediaFailure`. */
export class RoomFailure extends Error {
	readonly kind: MediaFailureKind | 'other';
	constructor(kind: RoomFailure['kind'], message: string) {
		super(message);
		this.name = 'RoomFailure';
		this.kind = kind;
	}
}

function classifyDeviceError(error: unknown): RoomFailure {
	const failure = MediaDeviceFailure.getFailure(error);
	if (failure === MediaDeviceFailure.PermissionDenied) {
		return new RoomFailure('permission_denied', 'Camera or microphone permission was denied.');
	}
	if (failure === MediaDeviceFailure.NotFound || failure === MediaDeviceFailure.DeviceInUse) {
		return new RoomFailure('no_device', 'No usable camera or microphone was found.');
	}
	return new RoomFailure('other', describeError(error));
}

function describeError(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

function trackSource(kind: TrackKind) {
	return kind === 'audio' ? Track.Source.Microphone : Track.Source.Camera;
}

function toParticipantInfo(participant: Participant | LocalParticipant, isLocal: boolean): ParticipantInfo {
	return {
		id: participant.identity,
		name: participant.name || participant.identity,
		isLocal,
		micEnabled: participant.isMicrophoneEnabled,
		cameraEnabled: participant.isCameraEnabled
	};
}

/** The subset of a connected LiveKit room `client.ts` needs. */
export interface RoomLike {
	connect(url: string, token: string): Promise<void>;
	disconnect(): Promise<void>;
	setMicrophoneEnabled(enabled: boolean): Promise<void>;
	setCameraEnabled(enabled: boolean): Promise<void>;
	switchActiveDevice(kind: DeviceKind, deviceId: string): Promise<void>;
	participants(): ParticipantInfo[];
	attach(participantId: string, kind: TrackKind, element: HTMLMediaElement): boolean;
	detach(participantId: string, kind: TrackKind, element: HTMLMediaElement): void;
	onDisconnected(callback: () => void): void;
	onParticipantsChanged(callback: () => void): void;
}

/** Wraps a real `livekit-client` `Room` as a `RoomLike`. */
export function wrapLiveKitRoom(room: Room = new Room()): RoomLike {
	const findParticipant = (participantId: string): Participant | LocalParticipant | undefined =>
		room.localParticipant.identity === participantId
			? room.localParticipant
			: room.remoteParticipants.get(participantId);

	return {
		async connect(url, token) {
			try {
				await room.connect(url, token);
			} catch (error) {
				throw new RoomFailure('token_refused', describeError(error));
			}
		},
		async disconnect() {
			await room.disconnect();
		},
		async setMicrophoneEnabled(enabled) {
			try {
				await room.localParticipant.setMicrophoneEnabled(enabled);
			} catch (error) {
				throw classifyDeviceError(error);
			}
		},
		async setCameraEnabled(enabled) {
			try {
				await room.localParticipant.setCameraEnabled(enabled);
			} catch (error) {
				throw classifyDeviceError(error);
			}
		},
		async switchActiveDevice(kind, deviceId) {
			try {
				await room.switchActiveDevice(kind, deviceId);
			} catch (error) {
				throw classifyDeviceError(error);
			}
		},
		participants() {
			const list = [toParticipantInfo(room.localParticipant, true)];
			for (const participant of room.remoteParticipants.values()) {
				list.push(toParticipantInfo(participant, false));
			}
			return list;
		},
		attach(participantId, kind, element) {
			const track = findParticipant(participantId)?.getTrackPublication(trackSource(kind))?.track;
			if (!track) return false;
			track.attach(element);
			return true;
		},
		detach(participantId, kind, element) {
			findParticipant(participantId)?.getTrackPublication(trackSource(kind))?.track?.detach(element);
		},
		onDisconnected(callback) {
			room.on(RoomEvent.Disconnected, callback);
		},
		onParticipantsChanged(callback) {
			room.on(RoomEvent.ParticipantConnected, callback);
			room.on(RoomEvent.ParticipantDisconnected, callback);
			room.on(RoomEvent.TrackMuted, callback);
			room.on(RoomEvent.TrackUnmuted, callback);
			room.on(RoomEvent.TrackSubscribed, callback);
			room.on(RoomEvent.TrackUnsubscribed, callback);
		}
	};
}

/**
 * Lists local devices of one kind. Passes `requestPermissions: false` so that merely listing
 * devices (for example to populate a picker before joining) never itself prompts for camera or
 * microphone access — only `join()`/`enableCamera()` do that.
 */
export async function listLocalDevices(kind: DeviceKind): Promise<MediaDeviceOption[]> {
	const devices = await Room.getLocalDevices(kind, false);
	return devices.map((device, index) => ({
		deviceId: device.deviceId,
		label: device.label || `${kind === 'audioinput' ? 'Microphone' : 'Camera'} ${index + 1}`
	}));
}
