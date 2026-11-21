import { describe, expect, it } from 'vitest';
import { track, type TrackParams } from './track';

function fakeMedia() {
	const calls: string[] = [];
	return {
		calls,
		media: {
			attachTrack: (id: string, kind: string) => void calls.push(`attach ${id} ${kind}`),
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
});
