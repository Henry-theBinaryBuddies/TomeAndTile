import { describe, expect, it, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { RollMode, TargetShape } from '@tomeandtile/contracts';
import {
	beginTargeting,
	endTargeting,
	endTargetingOnEscape,
	highlightIds,
	mapPickingFor,
	pickOnMap,
	targetingSession,
	updateTargeting
} from './session';
import { pickToken, removePick, setMode, type TargetingState } from './targeting';

function state(over: Partial<TargetingState> = {}): TargetingState {
	return {
		abilityId: 'a',
		abilityName: 'Spear',
		shape: TargetShape.CREATURES,
		picks: [],
		mode: RollMode.NORMAL,
		ordered: false,
		limit: 1,
		targetText: 'Up to 1 target',
		...over
	};
}

function key(k: string) {
	const calls = { prevented: 0, stopped: 0 };
	return {
		event: {
			key: k,
			preventDefault: () => void calls.prevented++,
			stopPropagation: () => void calls.stopped++
		},
		calls
	};
}

beforeEach(() => endTargeting());

describe('the shared picking session', () => {
	it('is empty until targeting begins, and empty again after it ends', () => {
		expect(get(targetingSession)).toBeNull();
		beginTargeting('c1', state());
		expect(get(targetingSession)?.creatureId).toBe('c1');
		endTargeting();
		expect(get(targetingSession)).toBeNull();
	});

	it('adds a token clicked on the map by the same rule as the list', () => {
		beginTargeting('c1', state());
		pickOnMap('t1');
		pickOnMap('t2');
		expect(get(targetingSession)?.state.picks).toEqual(['t1', 't2']);
		pickOnMap('t1');
		expect(get(targetingSession)?.state.picks).toEqual(['t2']);
	});

	it('never blocks a pick over the normal limit', () => {
		beginTargeting('c1', state({ limit: 1 }));
		pickOnMap('t1');
		pickOnMap('t2');
		pickOnMap('t3');
		expect(get(targetingSession)?.state.picks).toEqual(['t1', 't2', 't3']);
	});

	it('lets an ordered (multi-attack) ability pick the same token again', () => {
		beginTargeting('c1', state({ ordered: true, limit: 2 }));
		pickOnMap('t1');
		pickOnMap('t1');
		expect(get(targetingSession)?.state.picks).toEqual(['t1', 't1']);
	});

	it('shows a change from the list and a change from the map in the same picks', () => {
		beginTargeting('c1', state());
		updateTargeting((s) => pickToken(s, 'from-list'));
		pickOnMap('from-map');
		expect(get(targetingSession)?.state.picks).toEqual(['from-list', 'from-map']);
		updateTargeting((s) => removePick(s, 0));
		expect(get(targetingSession)?.state.picks).toEqual(['from-map']);
		updateTargeting((s) => setMode(s, RollMode.ADVANTAGE));
		expect(get(targetingSession)?.state.mode).toBe(RollMode.ADVANTAGE);
	});

	it('ignores map picks and list changes when nothing is in progress', () => {
		pickOnMap('t1');
		updateTargeting((s) => pickToken(s, 't1'));
		expect(get(targetingSession)).toBeNull();
	});

	it('ignores a map pick for an ability that picks nothing (self)', () => {
		beginTargeting('c1', state({ shape: TargetShape.SELF }));
		pickOnMap('t1');
		expect(get(targetingSession)?.state.picks).toEqual([]);
	});
});

describe('what the map is told', () => {
	it('is off with nothing highlighted when no picking is in progress', () => {
		expect(mapPickingFor(null)).toEqual({ enabled: false, highlights: [] });
	});

	it('turns picking on and rings each picked token once, in pick order', () => {
		const s = state({ ordered: true, picks: ['b', 'a', 'b'] });
		expect(highlightIds(s)).toEqual(['b', 'a']);
		expect(mapPickingFor({ creatureId: 'c', state: s })).toEqual({
			enabled: true,
			highlights: ['b', 'a']
		});
	});

	it('leaves the map as it is for an ability with no targets to pick', () => {
		expect(
			mapPickingFor({ creatureId: 'c', state: state({ shape: TargetShape.SELF, picks: ['x'] }) })
		).toEqual({ enabled: false, highlights: [] });
	});
});

describe('Escape', () => {
	it('ends picking and stops the key, so a phone panel stays open the first time', () => {
		beginTargeting('c1', state());
		const { event, calls } = key('Escape');
		expect(endTargetingOnEscape(event)).toBe(true);
		expect(calls).toEqual({ prevented: 1, stopped: 1 });
		expect(get(targetingSession)).toBeNull();
	});

	it('leaves Escape alone when nothing is being picked, so the panel can close', () => {
		const { event, calls } = key('Escape');
		expect(endTargetingOnEscape(event)).toBe(false);
		expect(calls).toEqual({ prevented: 0, stopped: 0 });
	});

	it('ignores other keys', () => {
		beginTargeting('c1', state());
		const { event } = key('Enter');
		expect(endTargetingOnEscape(event)).toBe(false);
		expect(get(targetingSession)).not.toBeNull();
	});
});
