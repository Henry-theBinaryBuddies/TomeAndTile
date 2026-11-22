// Public types for the video/voice client module (ADR 0007). None of these come from
// `livekit-client` — see `room.ts` for the only file in this module allowed to import it.

export type DeviceKind = 'audioinput' | 'videoinput';

export interface MediaDeviceOption {
	deviceId: string;
	label: string;
}

export type TrackKind = 'audio' | 'video';

/** Whether a track of one kind exists for a participant at all (`published`), and, if so,
 * whether it has actually arrived (`subscribed`). A tile tells "camera off" (not published)
 * from "waiting for video" (published but not yet subscribed) from these two booleans. For the
 * local participant `subscribed` mirrors `published`: there is no network subscription step for
 * your own track. */
export interface TrackAvailability {
	published: boolean;
	subscribed: boolean;
}

export interface ParticipantInfo {
	id: string;
	name: string;
	isLocal: boolean;
	micEnabled: boolean;
	cameraEnabled: boolean;
	mic: TrackAvailability;
	camera: TrackAvailability;
	isSpeaking: boolean;
}

export type MediaFailureKind =
	| 'permission_denied'
	| 'no_device'
	| 'token_refused'
	| 'connection_lost';

export interface MediaFailure {
	kind: MediaFailureKind;
	message: string;
}

export type MediaStatus = 'idle' | 'connecting' | 'joined' | 'leaving';

/** One device list per kind, refreshed on a `devicechange` event. The panel reads this instead
 * of polling `listDevices`. */
export interface DeviceLists {
	audioinput: MediaDeviceOption[];
	videoinput: MediaDeviceOption[];
}

/** The chosen device per kind, or `null` for "no explicit choice, use the default". Set by
 * `selectDevice` and kept even while idle, so a choice made before joining is remembered. */
export interface SelectedDeviceIds {
	audioinput: string | null;
	videoinput: string | null;
}

export interface MediaClientState {
	status: MediaStatus;
	/** From the last `MediaToken`. Spectators (`false`) never get a camera or microphone. */
	canPublish: boolean;
	micEnabled: boolean;
	cameraEnabled: boolean;
	participants: ParticipantInfo[];
	/** The most recent failure, if any. Cleared on the next successful join. */
	failure: MediaFailure | null;
	devices: DeviceLists;
	selectedDeviceId: SelectedDeviceIds;
	/** True when the browser has blocked remote audio playback (a common autoplay policy).
	 * `startAudioPlayback()` resumes it from a user gesture (a click handler, for example). */
	audioPlaybackBlocked: boolean;
}
