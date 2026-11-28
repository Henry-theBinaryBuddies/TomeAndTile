import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	Role,
	RollMode,
	SetHitPointsIntentSchema,
	TokenSchema,
	UndoIntentSchema,
	UseAbilityIntentSchema
} from '@tomeandtile/contracts';
import {
	REROLL_UNKNOWN,
	buildSetHitPointsKind,
	buildUndoKind,
	buildUseAgainKind,
	halvePrefill,
	hpPrefill,
	parseHp,
	rememberUse,
	rememberedUse,
	rerollBlockedReason,
	rerollNext,
	showOverrides,
	undoQuestion,
	undoSeq
} from './overrides';

const token = (over: Parameters<typeof create<typeof TokenSchema>>[1] = {}) =>
	create(TokenSchema, { tokenId: 't1', name: 'Goblin', ...over });

describe('who sees the buttons', () => {
	it('the GM sees them on a live card, nobody else does, and not on an undone card', () => {
		expect(showOverrides(Role.GM, false)).toBe(true);
		expect(showOverrides(Role.GM, true)).toBe(false);
		expect(showOverrides(Role.PLAYER, false)).toBe(false);
		expect(showOverrides(Role.UNSPECIFIED, false)).toBe(false);
	});
});

describe('undo', () => {
	it('undoes the first event of the use: the lowest seq on the card', () => {
		expect(undoSeq([12n, 10n, 11n])).toBe(10n);
		const kind = buildUndoKind([12n, 10n, 11n])!;
		expect(kind.case).toBe('undo');
		expect(kind.value).toEqual(create(UndoIntentSchema, { eventSeq: 10n }));
	});

	it('sends nothing for a card with no seq', () => {
		expect(undoSeq([])).toBeNull();
		expect(buildUndoKind([])).toBeNull();
	});

	it('names the ability, the source and each target once', () => {
		expect(undoQuestion('Longsword', 'Brannoch', ['Goblin', 'Orc', 'Goblin'])).toBe(
			'Undo Longsword by Brannoch on Goblin, Orc?'
		);
		expect(undoQuestion('Shield', 'Aria', [])).toBe('Undo Shield by Aria?');
	});
});

describe('set hit points', () => {
	it('sends current and temporary hit points for the token', () => {
		const kind = buildSetHitPointsKind('t1', 7, 2);
		expect(kind.case).toBe('setHitPoints');
		expect(kind.value).toEqual(create(SetHitPointsIntentSchema, { tokenId: 't1', hpCurrent: 7, hpTemp: 2 }));
	});

	it('accepts whole numbers from 0, with spaces', () => {
		expect(parseHp('7', '0')).toEqual({ ok: true, current: 7, temp: 0 });
		expect(parseHp(' 12 ', ' 3 ')).toEqual({ ok: true, current: 12, temp: 3 });
	});

	it('never accepts empty, non-numeric, negative or fractional input', () => {
		for (const bad of ['', ' ', 'x', '-1', '1.5', '1e2', '+3', '9999999']) {
			expect(parseHp(bad, '0').ok, `current ${bad}`).toBe(false);
			expect(parseHp('5', bad).ok, `temp ${bad}`).toBe(false);
		}
		expect(parseHp('', '0')).toMatchObject({ ok: false, reason: expect.stringMatching(/^Current HP/) });
		expect(parseHp('5', '')).toMatchObject({ ok: false, reason: expect.stringMatching(/^Temporary HP/) });
	});

	it('starts from the token numbers where the GM has them, and blank where not', () => {
		expect(hpPrefill(token({ hpCurrent: 4, hpTemp: 1 }))).toEqual({ current: '4', temp: '1' });
		expect(hpPrefill(token({ hpCurrent: 4 }))).toEqual({ current: '4', temp: '' });
		expect(hpPrefill(token())).toEqual({ current: '', temp: '' });
		expect(hpPrefill(undefined)).toEqual({ current: '', temp: '' });
	});
});

describe('halve', () => {
	it('gives back the damage that would not have been dealt', () => {
		// 14 damage dealt, 7 would have been dealt, so 7 comes back: 3 + 7 = 10.
		const h = halvePrefill(token({ hpCurrent: 3, hpMax: 20, hpTemp: 0 }), 14);
		expect(h).toMatchObject({ ok: true, current: 10, giveBack: 7, total: 14, temp: '0' });
		expect(h.ok && h.note).toBe('Gives back 7 of 14 damage. Check it and change it if needed.');
	});

	it('rounds the half down, so an odd total gives back the larger part', () => {
		// 7 damage: half is 3, so 4 comes back.
		expect(halvePrefill(token({ hpCurrent: 5, hpMax: 30 }), 7)).toMatchObject({ ok: true, current: 9, giveBack: 4 });
	});

	it('never goes above the maximum, and says what it really gives back', () => {
		const h = halvePrefill(token({ hpCurrent: 18, hpMax: 20 }), 14);
		expect(h).toMatchObject({ ok: true, current: 20, giveBack: 2, total: 14 });
	});

	it('is off when the page does not know the token hit points', () => {
		for (const t of [undefined, token(), token({ hpCurrent: 3 }), token({ hpCurrent: 3, hpMax: 0 })]) {
			const h = halvePrefill(t, 10);
			expect(h.ok).toBe(false);
			expect(!h.ok && h.reason).toMatch(/needs this token/);
		}
	});

	it('is off when there is no damage to halve', () => {
		expect(halvePrefill(token({ hpCurrent: 3, hpMax: 20 }), null).ok).toBe(false);
		expect(halvePrefill(token({ hpCurrent: 3, hpMax: 20 }), 0).ok).toBe(false);
	});
});

describe('re-roll', () => {
	const use = { sourceTokenId: 't-src', abilityId: 'longsword', targetTokenIds: ['a', 'b'], mode: RollMode.ADVANTAGE };

	it('is off for a use this page did not send', () => {
		expect(rerollBlockedReason('never-sent')).toBe(REROLL_UNKNOWN);
		expect(rerollBlockedReason('')).toBe(REROLL_UNKNOWN);
		expect(rememberedUse('never-sent')).toBeNull();
	});

	it('remembers the source, ability, targets and mode of a use it sent', () => {
		rememberUse('i-1', use);
		expect(rerollBlockedReason('i-1')).toBe('');
		const again = buildUseAgainKind(rememberedUse('i-1')!);
		expect(again.case).toBe('useAbility');
		expect(again.value).toEqual(
			create(UseAbilityIntentSchema, {
				sourceTokenId: 't-src',
				abilityId: 'longsword',
				targetTokenIds: ['a', 'b'],
				mode: RollMode.ADVANTAGE
			})
		);
	});

	it('keeps its own copy, so later changes to the list do not change it', () => {
		const targets = ['a'];
		rememberUse('i-2', { ...use, targetTokenIds: targets });
		targets.push('z');
		expect(rememberedUse('i-2')?.targetTokenIds).toEqual(['a']);
	});

	it('does not remember a use that was not sent', () => {
		rememberUse(null, use);
		expect(rememberedUse('null')).toBeNull();
	});

	it('sends nothing more when the undo was refused', () => {
		expect(rerollNext(true, false)).toBe('stop');
		expect(rerollNext(true, true)).toBe('stop');
	});

	it('waits until the server says the card is undone, then sends the use again', () => {
		expect(rerollNext(false, false)).toBe('wait');
		expect(rerollNext(false, true)).toBe('send_again');
	});
});
