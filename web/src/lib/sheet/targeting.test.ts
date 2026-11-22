import { describe, expect, it } from 'vitest';
import { RollMode, TargetShape } from '@tomeandtile/contracts';
import {
	buildUseAbilityKind,
	keepTargeting,
	limitWarning,
	pickCount,
	pickToken,
	picksTargets,
	removePick,
	setMode,
	startTargeting
} from './targeting';
import type { AbilityView } from './view';

const ability = (over: Partial<AbilityView> = {}): AbilityView => ({
	abilityId: 'longsword',
	name: 'Longsword',
	description: '',
	cost: 1,
	kind: 'usable',
	note: '',
	rangeText: '5 ft',
	targetText: 'Up to 1 target',
	resourceText: '',
	stepLines: [],
	shape: TargetShape.CREATURES,
	attackSteps: 1,
	maxTargets: 1,
	recharging: false,
	...over
});

describe('picking targets for one attack', () => {
	it('picks a token, and takes the pick back on a second click', () => {
		let s = startTargeting(ability());
		s = pickToken(s, 'gob');
		expect(s.picks).toEqual(['gob']);
		s = pickToken(s, 'gob');
		expect(s.picks).toEqual([]);
	});

	it('allows any number of picks, in the order picked', () => {
		let s = startTargeting(ability());
		for (const id of ['c', 'a', 'b', 'd']) s = pickToken(s, id);
		expect(s.picks).toEqual(['c', 'a', 'b', 'd']);
	});

	it('warns when over the limit but still builds the request', () => {
		let s = startTargeting(ability());
		s = pickToken(pickToken(s, 'a'), 'b');
		expect(limitWarning(s)).toBe(
			'Longsword normally has 1 target, and you picked 2. Every target is still used, and the server adds a warning. You can still confirm; the server decides what happens.'
		);
		const kind = buildUseAbilityKind('t1', s);
		expect(kind.case === 'useAbility' && kind.value.targetTokenIds).toEqual(['a', 'b']);
	});

	it('does not warn at or under the limit', () => {
		let s = startTargeting(ability());
		expect(limitWarning(s)).toBe('');
		s = pickToken(s, 'a');
		expect(limitWarning(s)).toBe('');
	});

	it('does not warn when the ability states no limit', () => {
		let s = startTargeting(ability({ maxTargets: 0 }));
		for (const id of ['a', 'b', 'c']) s = pickToken(s, id);
		expect(limitWarning(s)).toBe('');
	});

	it('can be confirmed with no targets (the server decides)', () => {
		const kind = buildUseAbilityKind('t1', startTargeting(ability()));
		expect(kind.case === 'useAbility' && kind.value.targetTokenIds).toEqual([]);
	});
});

describe('picking targets for a multi-attack', () => {
	const multi = () => startTargeting(ability({ name: 'Multiattack', attackSteps: 2 }));

	it('takes one pick per click, in order, and allows the same token again', () => {
		let s = multi();
		expect(s.ordered).toBe(true);
		s = pickToken(pickToken(s, 'gob'), 'gob');
		expect(s.picks).toEqual(['gob', 'gob']);
		expect(pickCount(s, 'gob')).toBe(2);
	});

	it('removes the pick at a position', () => {
		let s = multi();
		s = pickToken(pickToken(pickToken(s, 'a'), 'b'), 'c');
		s = removePick(s, 1);
		expect(s.picks).toEqual(['a', 'c']);
	});

	it('warns about more picks than attacks, without blocking', () => {
		let s = multi();
		for (const id of ['a', 'b', 'c']) s = pickToken(s, id);
		expect(limitWarning(s)).toContain('has 2 attacks, and you picked 3');
		expect(buildUseAbilityKind('t1', s).case).toBe('useAbility');
	});
});

describe('mode and the request', () => {
	it('sends the source token, ability, targets in pick order and the chosen mode', () => {
		let s = startTargeting(ability());
		s = pickToken(pickToken(s, 'x'), 'y');
		s = setMode(s, RollMode.ADVANTAGE);
		const kind = buildUseAbilityKind('token-brannoch', s);
		expect(kind.case).toBe('useAbility');
		if (kind.case !== 'useAbility') return;
		expect(kind.value.sourceTokenId).toBe('token-brannoch');
		expect(kind.value.abilityId).toBe('longsword');
		expect(kind.value.targetTokenIds).toEqual(['x', 'y']);
		expect(kind.value.mode).toBe(RollMode.ADVANTAGE);
	});

	it('starts in normal mode and can be set to disadvantage', () => {
		const s = startTargeting(ability());
		expect(s.mode).toBe(RollMode.NORMAL);
		expect(setMode(s, RollMode.DISADVANTAGE).mode).toBe(RollMode.DISADVANTAGE);
	});

	it('uses a self ability on itself: no targets can be picked or sent', () => {
		let s = startTargeting(ability({ shape: TargetShape.SELF, maxTargets: 0 }));
		expect(picksTargets(s)).toBe(false);
		s = pickToken(s, 'gob');
		expect(s.picks).toEqual([]);
		const kind = buildUseAbilityKind('t1', s);
		expect(kind.case === 'useAbility' && kind.value.targetTokenIds).toEqual([]);
	});
});

describe('warnings say what the server will do', () => {
	const multi = (attacks: number) => startTargeting(ability({ name: 'Multiattack', attackSteps: attacks }));

	it('multi-attack with too few picks: the later attacks will not be rolled', () => {
		const s = pickToken(multi(3), 'a');
		expect(limitWarning(s)).toBe(
			'Multiattack has 3 attacks, and you picked 1. The last 2 attacks will not be rolled. You can still confirm; the server decides what happens.'
		);
		expect(limitWarning(pickToken(pickToken(multi(3), 'a'), 'b'))).toContain('The last attack will not be rolled.');
	});

	it('multi-attack with no picks yet says nothing extra (the hint already asks for picks)', () => {
		expect(limitWarning(multi(2))).toBe('');
	});

	it('multi-attack with the right number of picks says nothing', () => {
		expect(limitWarning(pickToken(pickToken(multi(2), 'a'), 'a'))).toBe('');
	});

	it('multi-attack with too many picks: the extra ones are ignored', () => {
		let s = multi(2);
		for (const id of ['a', 'b', 'c']) s = pickToken(s, id);
		expect(limitWarning(s)).toBe(
			'Multiattack has 2 attacks, and you picked 3. The extra pick is ignored. You can still confirm; the server decides what happens.'
		);
		s = pickToken(s, 'd');
		expect(limitWarning(s)).toContain('The extra picks are ignored.');
	});

	it('one attack with too many targets: all are still used, with a warning from the server', () => {
		let s = startTargeting(ability({ maxTargets: 2 }));
		for (const id of ['a', 'b', 'c']) s = pickToken(s, id);
		expect(limitWarning(s)).toContain('normally has 2 targets, and you picked 3');
		expect(limitWarning(s)).toContain('Every target is still used, and the server adds a warning.');
	});

	it('never blocks a confirm for any of these', () => {
		const s = pickToken(multi(3), 'a');
		expect(buildUseAbilityKind('t1', s).case).toBe('useAbility');
	});
});

describe('keeping targeting open', () => {
	it('keeps it while the same character is shown, so an unrelated update does not end it', () => {
		expect(keepTargeting('creature-brannoch', 'creature-brannoch')).toBe(true);
	});

	it('ends it when another character is shown, or none', () => {
		expect(keepTargeting('creature-brannoch', 'creature-tamsin')).toBe(false);
		expect(keepTargeting('creature-brannoch', '')).toBe(false);
	});
});
