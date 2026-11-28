import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	CombatantSchema,
	EndEncounterIntentSchema,
	EndTurnIntentSchema,
	Role,
	SetInitiativeIntentSchema,
	StartEncounterIntentSchema,
	TokenSchema
} from '@tomeandtile/contracts';
import {
	NOTHING_TICKED,
	buildEndEncounterKind,
	buildEndTurnKind,
	buildSetInitiativeKind,
	buildStartKind,
	canControlEncounter,
	canEndTurn,
	canMove,
	creatureIds,
	moveChanges,
	parseInitiative,
	startBlockedReason,
	startChoices
} from './controls';

const token = (tokenId: string, name: string, over: Parameters<typeof create<typeof TokenSchema>>[1] = {}) =>
	create(TokenSchema, { tokenId, name, sceneId: 's1', creatureId: `c-${tokenId}`, ...over });
const tokens = {
	b: token('b', 'Goblin'),
	a: token('a', 'Aria'),
	prop: token('prop', 'Barrel', { creatureId: '' }),
	far: token('far', 'Elsewhere', { sceneId: 's2' })
};
const choices = startChoices(tokens, 's1');
const comb = (tokenId: string, initiative: number, order: number) =>
	create(CombatantSchema, { tokenId, initiative, order });

describe('start', () => {
	it('lists the active scene tokens by name', () => {
		expect(choices.map((c) => c.name)).toEqual(['Aria', 'Barrel', 'Goblin']);
		expect(choices.map((c) => c.isCreature)).toEqual([true, false, true]);
	});

	it('selects all creatures but not props', () => {
		expect(creatureIds(choices)).toEqual(['a', 'b']);
	});

	it('does not send when nothing is ticked, and says why', () => {
		expect(startBlockedReason([], 's1')).toBe(NOTHING_TICKED);
		expect(buildStartKind('s1', choices, [])).toBeNull();
	});

	it('does not send without an active map', () => {
		expect(startBlockedReason(['a'], '')).not.toBe('');
		expect(buildStartKind('', choices, ['a'])).toBeNull();
	});

	it('sends exactly the ticked ids, in checklist order', () => {
		const kind = buildStartKind('s1', choices, ['b', 'a'])!;
		expect(kind.case).toBe('startEncounter');
		expect(kind.value).toEqual(
			create(StartEncounterIntentSchema, { sceneId: 's1', tokenIds: ['a', 'b'] })
		);
	});

	it('ignores a ticked id that is no longer in the list', () => {
		const kind = buildStartKind('s1', choices, ['a', 'gone'])!;
		expect(kind.value).toMatchObject({ tokenIds: ['a'] });
		expect(buildStartKind('s1', choices, ['gone'])).toBeNull();
	});
});

describe('end turn and end encounter', () => {
	const current = token('a', 'Aria', { controllerUserIds: ['u-aria'] });

	it('is shown to the GM and to a controller of the current token', () => {
		expect(canEndTurn(Role.GM, 'gm', current)).toBe(true);
		expect(canEndTurn(Role.PLAYER, 'u-aria', current)).toBe(true);
	});

	it('is not shown to anyone else', () => {
		expect(canEndTurn(Role.PLAYER, 'u-other', current)).toBe(false);
		expect(canEndTurn(Role.PLAYER, '', token('x', 'X', { controllerUserIds: [''] }))).toBe(false);
		expect(canEndTurn(Role.PLAYER, 'u-aria', undefined)).toBe(false);
	});

	it('only the GM controls the encounter', () => {
		expect(canControlEncounter(Role.GM)).toBe(true);
		expect(canControlEncounter(Role.PLAYER)).toBe(false);
	});

	it('builds the intents with the encounter id', () => {
		expect(buildEndTurnKind('e1').value).toEqual(create(EndTurnIntentSchema, { encounterId: 'e1' }));
		const end = buildEndEncounterKind('e1');
		expect(end.case).toBe('endEncounter');
		expect(end.value).toEqual(create(EndEncounterIntentSchema, { encounterId: 'e1' }));
	});
});

describe('set initiative input', () => {
	it('accepts whole numbers, signed or padded', () => {
		expect(parseInitiative('15')).toEqual({ ok: true, value: 15 });
		expect(parseInitiative(' -2 ')).toEqual({ ok: true, value: -2 });
		expect(parseInitiative('+7')).toEqual({ ok: true, value: 7 });
	});

	it('never accepts empty, non-numeric, fractional or huge input', () => {
		for (const bad of ['', '   ', 'abc', '1.5', '1e3', '0x10', '12a', '99999999999', '--1'])
			expect(parseInitiative(bad).ok, bad).toBe(false);
	});

	it('builds one SetInitiativeIntent', () => {
		expect(buildSetInitiativeKind('e1', 'a', 18, 2).value).toEqual(
			create(SetInitiativeIntentSchema, { encounterId: 'e1', tokenId: 'a', initiative: 18, order: 2 })
		);
	});
});

describe('moving among equal initiatives', () => {
	const order = [comb('a', 18, 0), comb('b', 12, 0), comb('c', 12, 0), comb('d', 5, 0)];

	it('only moves between equal initiatives', () => {
		expect(canMove(order, 0, 'down')).toBe(false);
		expect(canMove(order, 1, 'up')).toBe(false);
		expect(canMove(order, 1, 'down')).toBe(true);
		expect(canMove(order, 2, 'up')).toBe(true);
		expect(canMove(order, 2, 'down')).toBe(false);
		expect(canMove(order, 3, 'down')).toBe(false);
	});

	it('sends one change when only one tie-break number has to change', () => {
		expect(moveChanges(order, 2, 'up')).toEqual([{ tokenId: 'b', initiative: 12, order: 1 }]);
		expect(moveChanges(order, 1, 'down')).toEqual([{ tokenId: 'b', initiative: 12, order: 1 }]);
	});

	it('numbers the whole tied group when the old numbers differ', () => {
		const tied = [comb('a', 10, 3), comb('b', 10, 7), comb('c', 10, 9)];
		expect(moveChanges(tied, 2, 'up')).toEqual([
			{ tokenId: 'a', initiative: 10, order: 0 },
			{ tokenId: 'c', initiative: 10, order: 1 },
			{ tokenId: 'b', initiative: 10, order: 2 }
		]);
	});

	it('sends nothing where a move is not allowed', () => {
		expect(moveChanges(order, 0, 'up')).toEqual([]);
		expect(moveChanges(order, 3, 'down')).toEqual([]);
		expect(moveChanges(order, 0, 'down')).toEqual([]);
	});

	it('does not move a token that appears twice', () => {
		const twice = [comb('a', 10, 0), comb('b', 10, 1), comb('a', 10, 2)];
		expect(canMove(twice, 1, 'up')).toBe(false);
		expect(moveChanges(twice, 1, 'down')).toEqual([]);
	});
});
