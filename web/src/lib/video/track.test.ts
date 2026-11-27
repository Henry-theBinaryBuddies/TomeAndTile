import { describe, expect, it } from 'vitest';
import type { ParticipantInfo } from '../../../media/index';
import { audioTrackParams, track, videoTrackParams, type TrackParams } from './track';

function fakeMedia() {
	const calls: string[] = [];
	return {
		calls,
		media: {
			attachTrack: (id: string, kind: string) => {
				calls.push(`attach ${id} ${kind}`);
				return true;
			},
			detachTrack: (id: string, kind: string) => void calls.push(`detach ${id} ${kind}`)
		} as TrackParams['media']
	};
}
const element = {} as HTMLMediaElement;

describe('track action', () => {
	it('attaches on mount and detaches when the element goes away', () => {
		const { media, calls } = fakeMedia();
		const action = track(element, { media, participantId: 'p1', kind: 'video' });
		expect(calls).toEqual(['attach p1 video']);
		action.destroy();
		expect(calls).toEqual(['attach p1 video', 'detach p1 video']);
	});

	it('moves the element when the participant changes', () => {
		const { media, calls } = fakeMedia();
		const action = track(element, { media, participantId: 'p1', kind: 'audio' });
		action.update({ media, participantId: 'p2', kind: 'audio' });
		expect(calls).toEqual(['attach p1 audio', 'detach p1 audio', 'attach p2 audio']);
		action.destroy();
		expect(calls.at(-1)).toBe('detach p2 audio');
	});

	it('re-attaches when the version changes, for example a camera switched on', () => {
		const { media, calls } = fakeMedia();
		const action = track(element, { media, participantId: 'p1', kind: 'video', version: false });
		action.update({ media, participantId: 'p1', kind: 'video', version: true });
		expect(calls).toEqual(['attach p1 video', 'detach p1 video', 'attach p1 video']);
	});

	it('does nothing when nothing changed', () => {
		const { media, calls } = fakeMedia();
		const action = track(element, { media, participantId: 'p1', kind: 'video', version: 1 });
		action.update({ media, participantId: 'p1', kind: 'video', version: 1 });
		expect(calls).toEqual(['attach p1 video']);
	});

	it('attaches a track that was not there at mount once the participant info is refreshed', () => {
		// The media module rebuilds the participant list (new objects) on every refresh, and
		// the tiles pass the participant object as the version. At mount a remote track often
		// does not exist yet, so the first attach finds nothing; the refresh must try again.
		const found = new Set<string>();
		const calls: string[] = [];
		const media = {
			attachTrack: (id: string, kind: string) => {
				const wasFound = found.has(id);
				calls.push(`${id} ${kind} ${wasFound ? 'found' : 'nothing'}`);
				return wasFound;
			},
			detachTrack: () => {}
		} as TrackParams['media'];
		const first = { id: 'p1', cameraEnabled: false };
		const action = track(element, { media, participantId: 'p1', kind: 'video', version: first });
		found.add('p1');
		// The same object again: nothing to do.
		action.update({ media, participantId: 'p1', kind: 'video', version: first });
		expect(calls).toEqual(['p1 video nothing']);
		// A new participant object with the same content (a refresh): attach again, now found.
		action.update({ media, participantId: 'p1', kind: 'video', version: { ...first } });
		expect(calls).toEqual(['p1 video nothing', 'p1 video found']);
	});
});

// What the tiles really pass to the action (the same functions the components call), so these
// fail if a tile stops re-attaching when it should.
describe('what a tile re-attaches on', () => {
	const person = (over: Partial<ParticipantInfo> = {}): ParticipantInfo => ({
		id: 'p1',
		name: 'Gale',
		isLocal: false,
		micEnabled: true,
		cameraEnabled: false,
		mic: { published: true, subscribed: true },
		camera: { published: false, subscribed: false },
		isSpeaking: false,
		...over
	});
	const call = { micEnabled: true, cameraEnabled: false };

	it('re-attaches video when someone else gets a new participant object', () => {
		const { media, calls } = fakeMedia();
		const p = person();
		const action = track(element, videoTrackParams(media, p, call));
		action.update(videoTrackParams(media, { ...p }, call));
		expect(calls).toEqual(['attach p1 video', 'detach p1 video', 'attach p1 video']);
	});

	it('leaves video alone when nothing about the participant or the call changed', () => {
		const { media, calls } = fakeMedia();
		const p = person();
		const action = track(element, videoTrackParams(media, p, call));
		action.update(videoTrackParams(media, p, call));
		expect(calls).toEqual(['attach p1 video']);
	});

	it('attaches your own video when your camera is turned on, though your participant object is the same', () => {
		// The media module does not rebuild the list when your own track is published, so
		// the participant object stays the same; the call state is what changes.
		const { media, calls } = fakeMedia();
		const me = person({ isLocal: true });
		const action = track(element, videoTrackParams(media, me, { micEnabled: true, cameraEnabled: false }));
		action.update(videoTrackParams(media, me, { micEnabled: true, cameraEnabled: true }));
		expect(calls).toEqual(['attach p1 video', 'detach p1 video', 'attach p1 video']);
	});

	it('re-attaches audio when a remote participant gets a new participant object', () => {
		const { media, calls } = fakeMedia();
		const p = person();
		const action = track(element, audioTrackParams(media, p));
		action.update(audioTrackParams(media, { ...p }));
		expect(calls).toEqual(['attach p1 audio', 'detach p1 audio', 'attach p1 audio']);
	});
});
