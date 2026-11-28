import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { CreatureSchema, Role, SetConditionsIntentSchema, TokenSchema } from '@tomeandtile/contracts';
import {
	CONDITIONS_UNKNOWN,
	buildSetConditionsKind,
	canEditConditions,
	closesEditor,
	conditionChanges,
	conditionDiff,
	conditionList,
	editorOptions,
	immuneNote,
	immunityList,
	shortList,
	toggleCondition
} from './view';
import { conditionName } from './names';

const token = (conditions: string[], name = 'Goblin') =>
	create(TokenSchema, { tokenId: 't1', name, conditions });
const creature = (over: Parameters<typeof create<typeof CreatureSchema>>[1] = {}) =>
	create(CreatureSchema, { creatureId: 'c1', name: 'Goblin', ...over });

describe('conditionList', () => {
	it('shows the token conditions by name', () => {
		const l = conditionList(token(['prone', 'poisoned']));
		expect(l.known).toBe(true);
		expect(l.text).toBe('Prone, Poisoned');
	});

	it('shows None for a token sent without conditions, even if the creature lists some', () => {
		const l = conditionList(token([]), creature({ conditions: ['prone'] }));
		expect(l.text).toBe('None');
		expect(l.items).toEqual([]);
	});

	it('falls back to the creature when there is no token', () => {
		expect(conditionList(undefined, creature({ conditions: ['blinded'] })).text).toBe('Blinded');
		expect(conditionList(undefined, creature()).text).toBe('None');
	});

	it('says "Conditions unknown" with no token and no creature', () => {
		const l = conditionList(undefined, undefined);
		expect(l.known).toBe(false);
		expect(l.text).toBe(CONDITIONS_UNKNOWN);
	});

	it('keeps an unknown id as sent', () => {
		expect(conditionList(token(['bless', 'x'])).text).toBe('bless, x');
	});
});

describe('immunityList and shortList', () => {
	it('lists immunities apart, by name', () => {
		const names = immunityList(creature({ conditionImmunities: ['prone', 'charmed'] })).map((i) => i.name);
		expect(names).toEqual(['Prone', 'Charmed']);
		expect(immunityList(undefined)).toEqual([]);
	});

	it('shortens a long list and keeps the full list', () => {
		const items = ['prone', 'blinded', 'stunned', 'bless'].map(conditionName);
		const s = shortList(items);
		expect(s.shown.map((i) => i.name)).toEqual(['Prone', 'Blinded']);
		expect(s.hidden).toBe(2);
		expect(s.full).toBe('Prone, Blinded, Stunned, bless');
	});

	it('does not shorten a short list', () => {
		const s = shortList(['prone'].map(conditionName));
		expect(s.hidden).toBe(0);
	});
});

describe('conditionChanges', () => {
	it('says what was added and removed, once each', () => {
		const before = { t1: token(['prone']) };
		const after = { t1: token(['poisoned']) };
		expect(conditionChanges(before, after)).toEqual([
			'Goblin is now Poisoned',
			'Goblin is no longer Prone'
		]);
	});

	it('says nothing when nothing changed or the token is new', () => {
		expect(conditionChanges({ t1: token(['prone']) }, { t1: token(['prone']) })).toEqual([]);
		expect(conditionChanges({}, { t1: token(['prone']) })).toEqual([]);
	});

	it('uses Unknown for an unnamed token', () => {
		expect(conditionChanges({ t1: token([], '') }, { t1: token(['prone'], '') })).toEqual([
			'Unknown is now Prone'
		]);
	});
});

describe('the GM editor', () => {
	it('offers every SRD condition and any other id the token has', () => {
		const ids = editorOptions(['prone', 'bless']).map((o) => o.id);
		expect(ids).toContain('blinded');
		expect(ids).toContain('unconscious');
		expect(ids).toContain('bless');
		expect(ids.filter((i) => i === 'prone')).toHaveLength(1);
		expect(ids).toHaveLength(16);
	});

	it('toggles an id on and off', () => {
		expect(toggleCondition(['prone'], 'blinded')).toEqual(['prone', 'blinded']);
		expect(toggleCondition(['prone', 'blinded'], 'prone')).toEqual(['blinded']);
	});

	it('works out ids to add and to remove', () => {
		expect(conditionDiff(['prone', 'bless'], ['prone', 'blinded'])).toEqual({
			add: ['blinded'],
			remove: ['bless']
		});
	});

	it('builds one SetConditionsIntent with the adds and removes', () => {
		const kind = buildSetConditionsKind('t1', ['prone'], ['blinded'])!;
		expect(kind.case).toBe('setConditions');
		expect(kind.value).toEqual(
			create(SetConditionsIntentSchema, { tokenId: 't1', add: ['blinded'], remove: ['prone'] })
		);
	});

	it('sends nothing when nothing changed', () => {
		expect(buildSetConditionsKind('t1', ['prone'], ['prone'])).toBeNull();
	});

	it('does not send an id that is in both lists', () => {
		const kind = buildSetConditionsKind('t1', ['prone'], ['prone', 'stunned'])!;
		expect(kind.value).toMatchObject({ add: ['stunned'], remove: [] });
	});

	it('notes immunity but does not block the choice', () => {
		const c = creature({ conditionImmunities: ['poisoned'] });
		expect(immuneNote('poisoned', c)).not.toBe('');
		expect(immuneNote('prone', c)).toBe('');
		const kind = buildSetConditionsKind('t1', [], ['poisoned'])!;
		expect(kind.value).toMatchObject({ add: ['poisoned'] });
	});

	it('is for the GM only', () => {
		expect(canEditConditions(Role.GM)).toBe(true);
		expect(canEditConditions(Role.PLAYER)).toBe(false);
		expect(canEditConditions(Role.UNSPECIFIED)).toBe(false);
	});

	it('closes on Esc, but not while composing text', () => {
		expect(closesEditor({ key: 'Escape' })).toBe(true);
		expect(closesEditor({ key: 'Escape', isComposing: true })).toBe(false);
		expect(closesEditor({ key: 'Enter' })).toBe(false);
	});
});
