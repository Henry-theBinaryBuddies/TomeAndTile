import { describe, expect, it, beforeEach } from 'vitest';
import { get, writable } from 'svelte/store';
import { Role, RollMode, TargetShape, type Token } from '@tomeandtile/contracts';
import { applySnapshot } from '../net/state';
import { loadPregenTable } from './fixtures';
import {
	beginTargeting,
	chosenTokenFor,
	endTargeting,
	endTargetingOnEscape,
	endTargetingWhenGone,
	followTargeting,
	highlightIds,
	mapPickingFor,
	pickOnMap,
	selectActingToken,
	selectCreature,
	selectedChoice,
	sheetSelection,
	targetingSession,
	updateTargeting
} from './session';
import { keepTargeting, pickToken, removePick, setMode, type TargetingState } from './targeting';
import { actingTokenId, sheetChoices } from './view';

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

beforeEach(() => {
	endTargeting();
	selectCreature('');
});

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

// The Sheet's body is unmounted when its panel is collapsed (or closed on a phone) and mounted
// again later. "Remounting" here is reading the shared store again, which is all a new panel
// body does to find out whose sheet to show.
describe('the Sheet\'s choices survive the panel being unmounted', () => {
	const player = () => ({ ...applySnapshot(loadPregenTable()), userId: 'mock-player', role: Role.PLAYER });

	it('shows the same character after reopening, not the first one', () => {
		const choices = sheetChoices(player());
		const second = choices[1];
		expect(choices[0].creatureId).not.toBe(second.creatureId);
		selectCreature(second.creatureId);
		// Start picking for that character, then the panel goes away and comes back.
		beginTargeting(second.creatureId, state());
		const reopened = selectedChoice(choices, get(sheetSelection));
		expect(reopened?.creatureId).toBe(second.creatureId);
		// Nothing about the picking changed, so the "switched character" check keeps it.
		expect(keepTargeting(get(targetingSession)!.creatureId, reopened!.creatureId)).toBe(true);
	});

	it('falls back to the first character when nothing is selected or it is gone', () => {
		const choices = sheetChoices(player());
		expect(selectedChoice(choices, get(sheetSelection))?.creatureId).toBe(choices[0].creatureId);
		selectCreature('creature-that-left');
		expect(selectedChoice(choices, get(sheetSelection))?.creatureId).toBe(choices[0].creatureId);
		expect(selectedChoice([], get(sheetSelection))).toBeUndefined();
	});

	it('keeps the token chosen to act, and acts with it after reopening', () => {
		const base = player();
		const first = base.tokens['token-brannoch'];
		const second: Token = { ...first, tokenId: 'token-brannoch-2', name: 'Brannoch, second' };
		const choices = sheetChoices({ ...base, tokens: { ...base.tokens, [second.tokenId]: second } });
		const brannoch = choices.find((c) => c.creatureId === 'creature-brannoch')!;
		expect(brannoch.tokens).toHaveLength(2);
		selectCreature(brannoch.creatureId);
		selectActingToken(brannoch.creatureId, second.tokenId);
		const chosen = chosenTokenFor(get(sheetSelection), brannoch.creatureId);
		expect(actingTokenId(brannoch, chosen)).toBe('token-brannoch-2');
	});

	it('forgets the token when another character is shown, and keeps it for the same one', () => {
		selectCreature('a');
		selectActingToken('a', 't2');
		selectCreature('a');
		expect(chosenTokenFor(get(sheetSelection), 'a')).toBe('t2');
		selectCreature('b');
		expect(chosenTokenFor(get(sheetSelection), 'b')).toBe('');
		expect(chosenTokenFor(get(sheetSelection), 'a')).toBe('');
	});
});

describe('picking ends when its character goes away, even with the Sheet closed', () => {
	const player = () => ({ ...applySnapshot(loadPregenTable()), userId: 'mock-player', role: Role.PLAYER });

	it('ends when a Snapshot removes the character', () => {
		const game = writable(player());
		const stop = endTargetingWhenGone(game);
		beginTargeting('creature-brannoch', state());
		game.update((g) => {
			const { 'creature-brannoch': _gone, ...rest } = g.creatures;
			return { ...g, creatures: rest };
		});
		expect(get(targetingSession)).toBeNull();
		stop();
	});

	it('keeps picking through updates that leave the character in place', () => {
		const game = writable(player());
		const stop = endTargetingWhenGone(game);
		beginTargeting('creature-brannoch', state());
		game.update((g) => ({ ...g }));
		const other = Object.keys(player().creatures).find((id) => id !== 'creature-brannoch')!;
		game.update((g) => {
			const { [other]: _gone, ...rest } = g.creatures;
			return { ...g, creatures: rest };
		});
		expect(get(targetingSession)?.creatureId).toBe('creature-brannoch');
		stop();
	});

	it('does nothing when no picking is in progress, and stops when told to', () => {
		const game = writable(player());
		const stop = endTargetingWhenGone(game);
		game.update((g) => ({ ...g, creatures: {} }));
		expect(get(targetingSession)).toBeNull();
		stop();
		beginTargeting('creature-brannoch', state());
		game.update((g) => ({ ...g, creatures: {} }));
		expect(get(targetingSession)).not.toBeNull();
	});
});

describe('what reaches the map as the session changes', () => {
	function fakeMap() {
		const calls: string[] = [];
		return {
			calls,
			map: {
				setPicking: (on: boolean) => void calls.push(`picking ${on}`),
				setHighlights: (ids: string[]) => void calls.push(`rings ${ids.join(',')}`)
			}
		};
	}

	it('turns picking and the rings on with the picks, and clears both when picking ends', () => {
		const { map, calls } = fakeMap();
		const stop = followTargeting(map);
		expect(calls).toEqual(['picking false', 'rings ']);
		calls.length = 0;
		beginTargeting('c1', state());
		pickOnMap('t1');
		pickOnMap('t2');
		expect(calls.slice(-2)).toEqual(['picking true', 'rings t1,t2']);
		endTargeting();
		expect(calls.slice(-2)).toEqual(['picking false', 'rings ']);
		stop();
		calls.length = 0;
		beginTargeting('c1', state());
		expect(calls).toEqual([]);
	});

	it('does not ring anything for an ability that picks nothing', () => {
		const { map, calls } = fakeMap();
		const stop = followTargeting(map);
		calls.length = 0;
		beginTargeting('c1', state({ shape: TargetShape.SELF }));
		expect(calls).toEqual(['picking false', 'rings ']);
		stop();
	});
});

describe('Escape, in detail', () => {
	it('does not notify anyone when it has nothing to do', () => {
		let notified = 0;
		const stop = targetingSession.subscribe(() => void notified++);
		beginTargeting('c1', state());
		const before = notified;
		endTargetingOnEscape(key('Enter').event);
		endTargetingOnEscape(key('a').event);
		expect(notified).toBe(before);
		endTargeting();
		endTargetingOnEscape(key('Escape').event);
		expect(notified).toBe(before + 1);
		stop();
	});

	it('leaves Escape to an input method that is composing text', () => {
		beginTargeting('c1', state());
		const { event, calls } = key('Escape');
		expect(endTargetingOnEscape({ ...event, isComposing: true })).toBe(false);
		expect(calls).toEqual({ prevented: 0, stopped: 0 });
		expect(get(targetingSession)).not.toBeNull();
	});
});
