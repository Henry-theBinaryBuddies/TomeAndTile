import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	AbilitySchema,
	AttackStepSchema,
	CreatureSchema,
	DieSchema,
	EffectStepSchema,
	HitOutcome,
	ResolutionSchema,
	ResolutionState,
	RollPurpose,
	RollResultSchema,
	TargetOutcomeSchema,
	TokenSchema,
	WarningSchema
} from '@tomeandtile/contracts';
import { rollAnnouncement } from './format';
import { outcomeView, resultCardView, warningText } from './result';

const roll = (total: number, label = '', notation = '1d20+5', sides = 20) =>
	create(RollResultSchema, {
		notation,
		label,
		total,
		dice: [create(DieSchema, { sides, value: Math.max(total - 5, 1) })]
	});

const tokens = {
	t1: create(TokenSchema, { tokenId: 't1', name: 'Brannoch', creatureId: 'c1' }),
	gob: create(TokenSchema, { tokenId: 'gob', name: 'Goblin' }),
	gob2: create(TokenSchema, { tokenId: 'gob2', name: '<img src=x onerror=alert(1)>' })
};

const outcome = (tokenId: string, kind: HitOutcome, over: NonNullable<Parameters<typeof create<typeof TargetOutcomeSchema>>[1]> = {}) =>
	create(TargetOutcomeSchema, { tokenId, outcome: kind, ...over });

const resolution = (over: NonNullable<Parameters<typeof create<typeof ResolutionSchema>>[1]> = {}) =>
	create(ResolutionSchema, {
		resolutionId: 'r1',
		sourceTokenId: 't1',
		abilityId: 'longsword',
		abilityName: 'Longsword',
		state: ResolutionState.DONE,
		...over
	});

const view = (r = resolution(), warnings: ReturnType<typeof warn>[] = [], creatures = {}) =>
	resultCardView(r, tokens, creatures, warnings);
const warn = (code: string) => create(WarningSchema, { code, message: 'server words' });

describe('resultCardView', () => {
	it('shows the ability name and the source token name', () => {
		const v = view();
		expect(v.abilityName).toBe('Longsword');
		expect(v.sourceName).toBe('Brannoch');
		expect(v.stateText).toBe('');
	});

	it('shows each target in order with its outcome in words, attack roll and damage rolls', () => {
		const v = view(
			resolution({
				outcomes: [
					outcome('gob', HitOutcome.HIT, {
						roll: roll(17, 'Longsword attack'),
						damageRolls: [roll(9, 'Slashing', '1d8+4', 8)]
					}),
					outcome('gob2', HitOutcome.CRITICAL_HIT, { roll: roll(25) }),
					outcome('gob', HitOutcome.MISS, { roll: roll(6) })
				]
			})
		);
		expect(v.targets.map((t) => t.name)).toEqual(['Goblin', '<img src=x onerror=alert(1)>', 'Goblin']);
		expect(v.targets.map((t) => t.outcome.text)).toEqual(['Hit', 'Critical hit', 'Miss']);
		expect(v.targets[0].roll?.title).toBe('Longsword attack');
		expect(v.targets[0].roll?.total).toBe(17);
		expect(v.targets[0].damageRolls.map((d) => d.total)).toEqual([9]);
		expect(v.targets[1].damageRolls).toEqual([]);
	});

	it('takes every number from the server and computes none', () => {
		// A hit with a damage roll whose total does not match its dice: shown as sent.
		const odd = create(RollResultSchema, {
			notation: '1d8+4',
			total: 99,
			dice: [create(DieSchema, { sides: 8, value: 1 })]
		});
		const v = view(resolution({ outcomes: [outcome('gob', HitOutcome.HIT, { roll: roll(3), damageRolls: [odd] })] }));
		expect(v.targets[0].damageRolls[0].total).toBe(99);
		expect(v.targets[0].roll?.total).toBe(3);
	});

	it('says the roll is unknown when the server sent none, never zero', () => {
		const v = view(resolution({ outcomes: [outcome('gob', HitOutcome.MISS)] }));
		expect(v.targets[0].roll).toBeNull();
		expect(v.targets[0].rollNote).toBe('Roll: unknown');
	});

	it('does not call a missing roll unknown when none was needed', () => {
		const v = view(resolution({ outcomes: [outcome('gob', HitOutcome.AUTOMATIC)] }));
		expect(v.targets[0].rollNote).toBe('');
		expect(v.targets[0].outcome.text).toBe('No roll needed');
	});

	it('says Unknown for a missing token or ability name, never a guess', () => {
		const v = resultCardView(
			resolution({ sourceTokenId: 'nobody', abilityName: ' ', outcomes: [outcome('ghost', HitOutcome.HIT)] }),
			tokens,
			{},
			[]
		);
		expect(v.sourceName).toBe('Unknown');
		expect(v.abilityName).toBe('Unknown ability');
		expect(v.targets[0].name).toBe('Unknown');
	});

	it('says an unspecified outcome is unknown', () => {
		expect(outcomeView(HitOutcome.UNSPECIFIED).text).toBe('Unknown');
		expect(outcomeView(HitOutcome.SAVE_FAILED).text).toBe('Save failed');
		expect(outcomeView(HitOutcome.SAVE_SUCCEEDED).text).toBe('Save succeeded');
	});

	it('shows the state while the resolution is not finished', () => {
		expect(view(resolution({ state: ResolutionState.TARGETING })).stateText).toBe('In progress');
		expect(view(resolution({ state: ResolutionState.DAMAGE })).stateText).toBe('In progress');
		expect(view(resolution({ state: ResolutionState.CANCELLED })).stateText).toBe('Cancelled');
	});

	it('says the state is unknown when the server sent none, instead of "In progress"', () => {
		expect(view(resolution({ state: ResolutionState.UNSPECIFIED })).stateText).toBe('State unknown');
	});

	it('keeps text from the server as plain text', () => {
		const v = view(resolution({ abilityName: '<b>Smite</b>', outcomes: [outcome('gob2', HitOutcome.HIT)] }));
		expect(v.abilityName).toBe('<b>Smite</b>');
		expect(v.targets[0].name).toBe('<img src=x onerror=alert(1)>');
	});
});

describe('multi-attack', () => {
	const steps = (n: number) =>
		Array.from({ length: n }, () =>
			create(EffectStepSchema, { step: { case: 'attack', value: create(AttackStepSchema, { toHitBonus: 5 }) } })
		);
	const multiCreature = create(CreatureSchema, {
		creatureId: 'c1',
		abilities: [create(AbilitySchema, { abilityId: 'longsword', steps: steps(2) })]
	});
	const single = create(CreatureSchema, {
		creatureId: 'c1',
		abilities: [create(AbilitySchema, { abilityId: 'longsword', steps: steps(1) })]
	});

	it('numbers attacks from step_index when outcomes come from different steps', () => {
		const v = view(
			resolution({
				outcomes: [
					outcome('gob', HitOutcome.HIT, { stepIndex: 0, roll: roll(15) }),
					outcome('gob', HitOutcome.MISS, { stepIndex: 1, roll: roll(4) })
				]
			})
		);
		expect(v.targets.map((t) => t.attackNumber)).toEqual([1, 2]);
		expect(v.announcement).toContain('attack 2 Goblin: Miss');
	});

	it('numbers a lone outcome when the sheet shows the ability has several attacks', () => {
		const v = view(
			resolution({ outcomes: [outcome('gob', HitOutcome.HIT, { stepIndex: 1, roll: roll(15) })] }),
			[],
			{ c1: multiCreature }
		);
		expect(v.targets[0].attackNumber).toBe(2);
	});

	it('shows no attack number for a single attack', () => {
		const v = view(resolution({ outcomes: [outcome('gob', HitOutcome.HIT, { roll: roll(15) })] }), [], {
			c1: single
		});
		expect(v.targets[0].attackNumber).toBeNull();
	});

	it('shows no attack number when several targets share one step', () => {
		const v = view(
			resolution({
				outcomes: [outcome('gob', HitOutcome.HIT), outcome('gob2', HitOutcome.MISS)]
			})
		);
		expect(v.targets.map((t) => t.attackNumber)).toEqual([null, null]);
	});
});

describe('warnings', () => {
	it('have plain words for the four ability codes', () => {
		const texts = ['out_of_range', 'too_many_targets', 'not_enough_resource', 'extra_targets_ignored'].map(warningText);
		expect(new Set(texts).size).toBe(4);
		for (const t of texts) expect(t).not.toMatch(/_/);
	});

	it('have a fallback for an unknown code that never shows the server message', () => {
		expect(warningText('brand_new')).toBe('The server added a note about this (brand_new).');
		expect(warningText('')).toContain('no reason given');
		expect(view(resolution(), [warn('brand_new')]).warnings.join()).not.toContain('server words');
	});

	it('come through in the card in order', () => {
		const v = view(resolution(), [warn('out_of_range'), warn('not_enough_resource')]);
		expect(v.warnings).toEqual([warningText('out_of_range'), warningText('not_enough_resource')]);
	});
});

describe('who rolled what', () => {
	const savingThrow = () =>
		create(RollResultSchema, {
			notation: '1d20+1',
			label: 'DEX save',
			total: 9,
			purpose: RollPurpose.SAVE,
			dice: [create(DieSchema, { sides: 20, value: 8 })]
		});
	const withOutcome = (kind: HitOutcome, roll = savingThrow()) =>
		view(resolution({ outcomes: [outcome('gob', kind, { roll })] }));

	it('names the target as the roller of a saving throw it failed', () => {
		const target = withOutcome(HitOutcome.SAVE_FAILED).targets[0];
		expect(target.rollWho).toBe('Goblin');
	});

	it('names the target as the roller of a saving throw it passed', () => {
		const target = withOutcome(HitOutcome.SAVE_SUCCEEDED).targets[0];
		expect(target.rollWho).toBe('Goblin');
	});

	it('goes by the roll\'s purpose when the outcome does not say it is a save', () => {
		const target = withOutcome(HitOutcome.UNSPECIFIED).targets[0];
		expect(target.rollWho).toBe('Goblin');
	});

	it('keeps the attacker as the roller of an attack roll', () => {
		const attack = roll(17, 'Attack');
		const hit = withOutcome(HitOutcome.HIT, attack).targets[0];
		const miss = withOutcome(HitOutcome.MISS, attack).targets[0];
		expect(hit.rollWho).toBe('Brannoch');
		expect(miss.rollWho).toBe('Brannoch');
	});

	it('says the target rolled in the screen-reader sentence for a save', () => {
		const target = withOutcome(HitOutcome.SAVE_FAILED).targets[0];
		const sentence = rollAnnouncement(target.rollWho, target.roll!);
		expect(sentence.startsWith('Goblin rolled')).toBe(true);
		expect(sentence).not.toContain('Brannoch');
	});

	it('shows an unknown target as Unknown, not as the attacker', () => {
		const save = view(
			resolution({ outcomes: [outcome('nobody', HitOutcome.SAVE_FAILED, { roll: savingThrow() })] })
		);
		expect(save.targets[0].rollWho).toBe('Unknown');
	});
});
