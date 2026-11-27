// Plain-language text and small decisions for the Video panel. No LiveKit here, and no
// game rules: it only describes what the media client reports. Text that came from a
// person or the server (a participant's name) is returned as text, never as HTML.
import type { MediaClientState, MediaFailureKind, MediaStatus, ParticipantInfo } from '../../../media/index';

const FAILURE_TEXT: Record<MediaFailureKind, string> = {
	permission_denied:
		'The browser did not allow the camera or microphone. Allow it in the site settings, then try again.',
	no_device: 'No camera or microphone was found. Plug one in, then try again.',
	// Also covers a server with video turned off (media_not_configured) and no reply at all.
	token_refused:
		'The server did not let you into the call. Video may not be set up on this server, or it did not answer. Try again later.',
	connection_lost: 'The call connection was lost. Try joining again.',
	joined_elsewhere: 'You joined the call from another tab or device.',
	removed: 'You were removed from the call.',
	room_closed: 'The call has ended.'
};

/** What to tell the person about a failure. The module's own message is never shown. */
export function failureText(kind: string): string {
	return (
		(FAILURE_TEXT as Record<string, string>)[kind] ?? 'Something went wrong with the call. Try again.'
	);
}

/** The label on the Join / Leave button. */
export function joinLabel(status: MediaStatus): string {
	switch (status) {
		case 'idle':
			return 'Join call';
		case 'connecting':
			return 'Joining…';
		case 'joined':
			return 'Leave call';
		case 'leaving':
			return 'Leaving…';
	}
}

/** Join and Leave only work while nothing is changing. */
export function joinDisabled(status: MediaStatus): boolean {
	return status === 'connecting' || status === 'leaving';
}

/** A short line for screen readers and the status area. Empty when idle. */
export function statusText(status: MediaStatus): string {
	switch (status) {
		case 'connecting':
			return 'Joining the call…';
		case 'joined':
			return 'You are in the call.';
		case 'leaving':
			return 'Leaving the call…';
		default:
			return '';
	}
}

export interface ControlView {
	/** The mic and camera buttons can be pressed. */
	enabled: boolean;
	/** Why they cannot, when that is something the person can understand. */
	reason: string;
}

/** Mute and camera buttons: only in a call, and never for a spectator (subscribe-only token). */
export function controlsView(state: Pick<MediaClientState, 'status' | 'canPublish'>): ControlView {
	if (state.status !== 'joined') return { enabled: false, reason: 'Join the call first.' };
	if (!state.canPublish) {
		return { enabled: false, reason: 'Spectators can watch and listen, but cannot use a camera or microphone.' };
	}
	return { enabled: true, reason: '' };
}

export function micButtonLabel(micEnabled: boolean): string {
	return micEnabled ? 'Turn microphone off' : 'Turn microphone on';
}

export function cameraButtonLabel(cameraEnabled: boolean): string {
	return cameraEnabled ? 'Turn camera off' : 'Turn camera on';
}

export type CameraStatus = 'on' | 'off' | 'waiting';

export interface TileView {
	id: string;
	name: string;
	micText: string;
	cameraText: string;
	/** `waiting` is a camera that is on but whose video has not arrived yet. */
	camera: CameraStatus;
	/** The person is talking right now. Shown in words as well as styling. */
	speaking: boolean;
	/** Read out for screen readers as one sentence. */
	summary: string;
}

/**
 * What a participant's tile shows. A missing name is "Unknown", never a guess. "Waiting for"
 * applies to other people only: it means they have turned the track on but it has not reached
 * this browser yet, which is different from it being off. Your own tracks need no network step.
 */
export function tileView(p: ParticipantInfo): TileView {
	const base = p.name.trim() === '' ? 'Unknown' : p.name.trim();
	const name = p.isLocal ? `${base} (you)` : base;
	const camera: CameraStatus = !p.cameraEnabled
		? 'off'
		: !p.isLocal && !p.camera.subscribed
			? 'waiting'
			: 'on';
	const micText = !p.micEnabled
		? 'Microphone off'
		: !p.isLocal && !p.mic.subscribed
			? 'Waiting for audio'
			: 'Microphone on';
	const cameraText = camera === 'off' ? 'Camera off' : camera === 'waiting' ? 'Waiting for video' : 'Camera on';
	const speaking = p.isSpeaking;
	return {
		id: p.id,
		name,
		micText,
		cameraText,
		camera,
		speaking,
		summary: `${name}. ${micText}. ${cameraText}.${speaking ? ' Speaking.' : ''}`
	};
}

/**
 * The local tile must match the Mic and Camera buttons. The media module rebuilds the
 * participant list only on remote and mute events, so for yourself the call's own mic and
 * camera state is the one to show. Other people are shown as reported.
 */
export function shownParticipant(
	p: ParticipantInfo,
	call: Pick<MediaClientState, 'micEnabled' | 'cameraEnabled'>
): ParticipantInfo {
	return p.isLocal ? { ...p, micEnabled: call.micEnabled, cameraEnabled: call.cameraEnabled } : p;
}

/** Remote participants only: the person's own microphone is never played back to them. */
export function audioParticipants(participants: readonly ParticipantInfo[]): ParticipantInfo[] {
	return participants.filter((p) => !p.isLocal);
}

/** Shown while the browser blocks sound from the call (autoplay policy). */
export const AUDIO_BLOCKED_TEXT = 'Your browser is blocking sound from the call.';
export const AUDIO_BLOCKED_BUTTON = 'Tap to enable sound';

export const DEFAULT_DEVICE_LABEL = 'Browser default';

/** A device's label, or a stand-in when the browser hides labels until permission is given. */
export function deviceLabel(label: string, index: number, kind: 'audioinput' | 'videoinput'): string {
	const trimmed = label.trim();
	if (trimmed) return trimmed;
	return `${kind === 'audioinput' ? 'Microphone' : 'Camera'} ${index + 1}`;
}
