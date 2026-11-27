// Attaches one participant's track to an element and detaches it again. A Svelte action:
// `use:track={{ media, participantId, kind, version }}`. The element is attached on mount,
// moved when the participant, the track kind or `version` changes, and detached when the
// element goes away. `attachTrack` says whether a track was there; callers pass a version that
// changes whenever the tile should try again (a new participant object from the module, or, for
// your own video, a change in what the tile shows).
import type { MediaClient, MediaClientState, ParticipantInfo, TrackKind } from '../../../media/index';
import { shownParticipant } from './view';

export interface TrackParams {
	media: Pick<MediaClient, 'attachTrack' | 'detachTrack'>;
	participantId: string;
	kind: TrackKind;
	/** Change this to re-attach, e.g. when a camera is switched on after the tile appeared. */
	version?: unknown;
}

export function track(element: HTMLMediaElement, initial: TrackParams) {
	let current = initial;
	current.media.attachTrack(current.participantId, current.kind, element);
	return {
		update(next: TrackParams) {
			if (
				next.media === current.media &&
				next.participantId === current.participantId &&
				next.kind === current.kind &&
				next.version === current.version
			) {
				return;
			}
			current.media.detachTrack(current.participantId, current.kind, element);
			current = next;
			current.media.attachTrack(current.participantId, current.kind, element);
		},
		destroy() {
			current.media.detachTrack(current.participantId, current.kind, element);
		}
	};
}

type CallLook = Pick<MediaClientState, 'micEnabled' | 'cameraEnabled'>;

/**
 * What a tile's video element passes to `track`. The version is what the tile shows
 * (`shownParticipant`), not the participant list entry: for yourself the list entry does not
 * change when your own camera is published, but what the tile shows does, and the video must be
 * attached again then. Attaching the same track twice is harmless.
 */
export function videoTrackParams(
	media: TrackParams['media'],
	participant: ParticipantInfo,
	call: CallLook
): TrackParams {
	return {
		media,
		participantId: participant.id,
		kind: 'video',
		version: shownParticipant(participant, call)
	};
}

/** What a remote participant's audio element passes to `track`. */
export function audioTrackParams(media: TrackParams['media'], participant: ParticipantInfo): TrackParams {
	return { media, participantId: participant.id, kind: 'audio', version: participant };
}
