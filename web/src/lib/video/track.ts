// Attaches one participant's track to an element and detaches it again. A Svelte action:
// `use:track={{ media, participantId, kind }}`. The element is attached on mount, moved when
// the participant or the track kind changes, and detached when the element goes away.
import type { MediaClient, TrackKind } from '../../../media/index';

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
