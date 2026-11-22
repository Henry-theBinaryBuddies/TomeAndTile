import { describe, expect, it } from 'vitest';
import type { MediaFailureKind, MediaStatus, ParticipantInfo } from '../../../media/index';
import {
	audioParticipants,
	cameraButtonLabel,
	controlsView,
	deviceLabel,
	failureText,
	joinDisabled,
	joinLabel,
	micButtonLabel,
	shownParticipant,
	statusText,
	tileView
} from './view';

const person = (over: Partial<ParticipantInfo> = {}): ParticipantInfo => ({
	id: 'p1',
	name: 'Gale',
	isLocal: false,
	micEnabled: true,
	cameraEnabled: false,
	...over
});

describe('failureText', () => {
	const kinds: MediaFailureKind[] = ['permission_denied', 'no_device', 'token_refused', 'connection_lost'];

	it('has its own plain words for each failure', () => {
		const seen = new Set(kinds.map(failureText));
		expect(seen.size).toBe(4);
		for (const kind of kinds) expect(failureText(kind)).not.toContain(kind);
	});

	it('covers a server with video off or silent under token_refused', () => {
		expect(failureText('token_refused')).toMatch(/not be set up|did not answer/);
	});

	it('never shows the module message and has a fallback for a new kind', () => {
		expect(failureText('something_new')).toBe('Something went wrong with the call. Try again.');
	});
});

describe('join button', () => {
	const cases: [MediaStatus, string, boolean][] = [
		['idle', 'Join call', false],
		['connecting', 'Joining…', true],
		['joined', 'Leave call', false],
		['leaving', 'Leaving…', true]
	];
	it.each(cases)('%s says %s (disabled: %s)', (status, label, disabled) => {
		expect(joinLabel(status)).toBe(label);
		expect(joinDisabled(status)).toBe(disabled);
	});

	it('announces only the states that change', () => {
		expect(statusText('idle')).toBe('');
		expect(statusText('connecting')).toMatch(/Joining/);
		expect(statusText('joined')).toMatch(/in the call/);
		expect(statusText('leaving')).toMatch(/Leaving/);
	});
});

describe('controlsView', () => {
	it('is off with a reason before joining', () => {
		const v = controlsView({ status: 'idle', canPublish: true });
		expect(v.enabled).toBe(false);
		expect(v.reason).toMatch(/Join the call/);
	});

	it('is off while joining or leaving', () => {
		expect(controlsView({ status: 'connecting', canPublish: true }).enabled).toBe(false);
		expect(controlsView({ status: 'leaving', canPublish: true }).enabled).toBe(false);
	});

	it('is on for someone who can publish', () => {
		expect(controlsView({ status: 'joined', canPublish: true })).toEqual({ enabled: true, reason: '' });
	});

	it('is off with an explanation for a spectator', () => {
		const v = controlsView({ status: 'joined', canPublish: false });
		expect(v.enabled).toBe(false);
		expect(v.reason).toMatch(/Spectators/);
	});

	it('labels the buttons by what pressing does', () => {
		expect(micButtonLabel(true)).toBe('Turn microphone off');
		expect(micButtonLabel(false)).toBe('Turn microphone on');
		expect(cameraButtonLabel(true)).toBe('Turn camera off');
		expect(cameraButtonLabel(false)).toBe('Turn camera on');
	});
});

describe('tileView', () => {
	it('shows the name and the mic and camera state', () => {
		const v = tileView(person());
		expect(v.name).toBe('Gale');
		expect(v.micText).toBe('Microphone on');
		expect(v.cameraText).toBe('Camera off');
		expect(v.summary).toBe('Gale. Microphone on. Camera off.');
	});

	it('marks the local tile', () => {
		expect(tileView(person({ isLocal: true })).name).toBe('Gale (you)');
	});

	it('says Unknown for a missing name, never a guess', () => {
		expect(tileView(person({ name: '   ' })).name).toBe('Unknown');
		expect(tileView(person({ name: '', isLocal: true })).name).toBe('Unknown (you)');
	});

	it('keeps markup as plain text', () => {
		const v = tileView(person({ name: '<img src=x onerror=alert(1)>' }));
		expect(v.name).toBe('<img src=x onerror=alert(1)>');
	});
});

describe('shownParticipant', () => {
	const call = { micEnabled: true, cameraEnabled: true };

	it('shows yourself as the call reports it, not as the last participant list said', () => {
		const stale = person({ isLocal: true, micEnabled: false, cameraEnabled: false });
		const shown = shownParticipant(stale, call);
		expect(shown.micEnabled).toBe(true);
		expect(shown.cameraEnabled).toBe(true);
		expect(tileView(shown).summary).toBe('Gale (you). Microphone on. Camera on.');
	});

	it('shows other people exactly as reported', () => {
		const other = person({ micEnabled: false, cameraEnabled: false });
		expect(shownParticipant(other, call)).toBe(other);
	});

	it('does not change the participant it was given', () => {
		const stale = person({ isLocal: true, micEnabled: false });
		shownParticipant(stale, call);
		expect(stale.micEnabled).toBe(false);
	});
});

describe('audioParticipants', () => {
	it('leaves out the local participant so your own voice is never played back', () => {
		const list = [person({ id: 'a', isLocal: true }), person({ id: 'b' }), person({ id: 'c' })];
		expect(audioParticipants(list).map((p) => p.id)).toEqual(['b', 'c']);
	});
});

describe('deviceLabel', () => {
	it('uses the browser label, or a numbered stand-in when labels are hidden', () => {
		expect(deviceLabel('USB Mic', 0, 'audioinput')).toBe('USB Mic');
		expect(deviceLabel('', 1, 'audioinput')).toBe('Microphone 2');
		expect(deviceLabel('  ', 0, 'videoinput')).toBe('Camera 1');
	});
});
