import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	AbilitySchema,
	ActionCost,
	CreatureSchema,
	Role,
	TargetShape,
	TargetingSchema,
	AbilityScoresSchema,
	ResourceSchema,
	EffectStepSchema,
	AttackStepSchema,
	DamagePartSchema,
	DamageType
} from '@tomeandtile/contracts';
import { applySnapshot } from '../net/state';
import { CHARACTER_FILES, loadCharacter, loadPregenTable } from './fixtures';
import { abilityGroups, abilityView, sheetChoices, signed, statBlockView } from './view';

const brannoch = () => loadCharacter('brannoch-vale-fighter');
const ability = (c: ReturnType<typeof brannoch>, id: string) =>
	abilityView(c.abilities.find((a) => a.abilityId === id)!, c);

describe('fixtures', () => {
	it('all five pregens parse and show a stat block and abilities', () => {
		expect(CHARACTER_FILES).toHaveLength(5);
		for (const file of CHARACTER_FILES) {
			const c = loadCharacter(file);
			const block = statBlockView(c);
			expect(block.name).not.toBe('Unknown');
			expect(block.scores).toHaveLength(6);
			expect(abilityGroups(c).flatMap((g) => g.abilities)).toHaveLength(c.abilities.length);
		}
	});
});

describe('statBlockView', () => {
	it('shows the numbers exactly as sent, with no modifiers worked out', () => {
		const b = statBlockView(brannoch());
		expect(b.name).toBe('Brannoch Vale');
		expect(b.stats).toContainEqual({ label: 'Hit points', value: '28 of 28' });
		expect(b.stats).toContainEqual({ label: 'Armor class', value: '19' });
		expect(b.stats).toContainEqual({ label: 'Speed', value: '30 ft' });
		expect(b.stats).toContainEqual({ label: 'Initiative', value: '+2' });
		expect(b.stats).toContainEqual({ label: 'Proficiency', value: '+2' });
		expect(b.scores).toEqual([
			{ label: 'STR', value: '17' },
			{ label: 'DEX', value: '14' },
			{ label: 'CON', value: '14' },
			{ label: 'INT', value: '10' },
			{ label: 'WIS', value: '12' },
			{ label: 'CHA', value: '8' }
		]);
		expect(b.lists).toContainEqual({ label: 'Saving throws', value: 'Strength, Constitution' });
	});

	it('shows temporary hit points when there are some', () => {
		const c = create(CreatureSchema, { name: 'X', hpCurrent: 5, hpMax: 9, hpTemp: 3 });
		expect(statBlockView(c).stats[0].value).toBe('5 of 9 (+3 temporary)');
	});

	it('shows unknown, never zero, for what the server did not send', () => {
		const b = statBlockView(create(CreatureSchema, { name: '  ' }));
		expect(b.name).toBe('Unknown');
		expect(b.scores).toBeNull();
		const byLabel = Object.fromEntries(b.stats.map((s) => [s.label, s.value]));
		expect(byLabel['Hit points']).toBe('Unknown');
		expect(byLabel['Armor class']).toBe('Unknown');
		expect(byLabel['Proficiency']).toBe('Unknown');
	});

	it('shows an ability score of zero as unknown', () => {
		const c = create(CreatureSchema, { scores: create(AbilityScoresSchema, { strength: 12 }) });
		const scores = statBlockView(c).scores!;
		expect(scores[0].value).toBe('12');
		expect(scores[1].value).toBe('Unknown');
	});

	it('lists resources as current of max', () => {
		expect(statBlockView(brannoch()).resources).toContainEqual({ label: 'Second Wind', value: '2 of 2' });
	});

	it('keeps text from the server as plain text', () => {
		const b = statBlockView(create(CreatureSchema, { name: '<img src=x onerror=alert(1)>', summary: '<b>hi</b>' }));
		expect(b.name).toBe('<img src=x onerror=alert(1)>');
		expect(b.summary).toBe('<b>hi</b>');
	});

	it('signs numbers', () => {
		expect(signed(3)).toBe('+3');
		expect(signed(0)).toBe('+0');
		expect(signed(-1)).toBe('-1');
	});
});

describe('abilities', () => {
	it('are grouped by action cost in a fixed order, keeping the server order inside a group', () => {
		const groups = abilityGroups(brannoch());
		expect(groups.map((g) => g.label)).toEqual(expect.arrayContaining(['Actions', 'Bonus actions']));
		const actions = groups.find((g) => g.cost === ActionCost.ACTION)!;
		expect(actions.abilities.map((a) => a.abilityId).slice(0, 2)).toEqual(['longsword', 'longbow']);
		const order = groups.map((g) => g.cost);
		expect(order).toEqual([...order].sort((a, b) => (a || 99) - (b || 99)));
	});

	it('show range, the step lines as sent, and a usable kind for an attack', () => {
		const c = brannoch();
		const sword = ability(c, 'longsword');
		expect(sword.kind).toBe('usable');
		expect(sword.rangeText).toBe('5 ft');
		expect(sword.stepLines).toEqual(['Attack +5 to hit, damage 1d8+3 slashing']);
		expect(sword.attackSteps).toBe(1);
		expect(sword.maxTargets).toBe(1);
		expect(ability(c, 'longbow').rangeText).toBe('150/600 ft');
	});

	it('show the resource and what is left', () => {
		const c = brannoch();
		expect(ability(c, 'second-wind').resourceText).toBe('Uses 1 Second Wind (2 of 2 left)');
		expect(ability(c, 'second-wind').rangeText).toBe('Self');
		expect(ability(c, 'longsword').resourceText).toBe('');
	});

	it('say the amount left is unknown when the named resource is not on the sheet', () => {
		const c = create(CreatureSchema, {
			resources: [create(ResourceSchema, { key: 'rage', label: 'Rage', current: 1, max: 2 })],
			abilities: [create(AbilitySchema, { abilityId: 'x', name: 'X', resourceKey: 'ki', resourceAmount: 1 })]
		});
		expect(abilityView(c.abilities[0], c).resourceText).toBe('Uses 1 (amount left unknown)');
	});

	it('treat a passive ability and one with no steps as plain text, not clickable', () => {
		const c = create(CreatureSchema, {
			abilities: [
				create(AbilitySchema, { abilityId: 'p', name: 'Keen Sight', cost: ActionCost.PASSIVE }),
				create(AbilitySchema, { abilityId: 'n', name: 'Flavor', cost: ActionCost.ACTION })
			]
		});
		expect(abilityView(c.abilities[0], c).kind).toBe('passive');
		expect(abilityView(c.abilities[1], c).kind).toBe('no_steps');
	});

	it('show an area ability disabled, with a note that areas come later', () => {
		const step = create(EffectStepSchema, {
			step: {
				case: 'attack',
				value: create(AttackStepSchema, { toHitBonus: 1, damage: [create(DamagePartSchema, { dice: '2d6', type: DamageType.FIRE })] })
			}
		});
		const c = create(CreatureSchema, {
			abilities: [
				create(AbilitySchema, {
					abilityId: 'f',
					name: 'Burning Hands',
					cost: ActionCost.ACTION,
					targeting: create(TargetingSchema, { shape: TargetShape.CONE, sizeFeet: 15 }),
					steps: [step]
				})
			]
		});
		const v = abilityView(c.abilities[0], c);
		expect(v.kind).toBe('area');
		expect(v.note).toBe('Area abilities come later.');
		expect(v.rangeText).toBe('Area, 15 ft');
		expect(v.stepLines[0]).toContain('2d6 fire');
	});

	it('count attack steps so a multi-attack is picked one target per step', () => {
		const attack = create(EffectStepSchema, { step: { case: 'attack', value: create(AttackStepSchema, { toHitBonus: 4 }) } });
		const c = create(CreatureSchema, {
			abilities: [
				create(AbilitySchema, {
					abilityId: 'multi',
					name: 'Multiattack',
					cost: ActionCost.ACTION,
					targeting: create(TargetingSchema, { shape: TargetShape.CREATURES, maxTargets: 1 }),
					steps: [attack, attack]
				})
			]
		});
		expect(abilityView(c.abilities[0], c).attackSteps).toBe(2);
	});
});

describe('sheetChoices', () => {
	const table = () => applySnapshot(loadPregenTable());

	it('gives mock-player all five pregens, each with its token', () => {
		const s = { ...table(), userId: 'mock-player', role: Role.PLAYER };
		const choices = sheetChoices(s);
		expect(choices).toHaveLength(5);
		expect(choices.map((c) => c.name)).toContain('Brannoch Vale');
		const b = choices.find((c) => c.creatureId === 'creature-brannoch')!;
		expect(b.sourceTokenId).toBe('token-brannoch');
	});

	it('gives a player nothing they neither own nor control', () => {
		const s = { ...table(), userId: 'someone-else', role: Role.PLAYER };
		expect(sheetChoices(s)).toEqual([]);
	});

	it('gives the GM every creature the server sent', () => {
		const s = { ...table(), userId: 'mock-gm', role: Role.GM };
		expect(sheetChoices(s).length).toBeGreaterThanOrEqual(6);
	});

	it('includes an owned creature that has no token yet, with no source token', () => {
		const s = {
			userId: 'u1',
			role: Role.PLAYER,
			creatures: { c1: create(CreatureSchema, { creatureId: 'c1', name: 'Mine', ownerUserId: 'u1' }) },
			tokens: {}
		};
		expect(sheetChoices(s)).toEqual([{ creatureId: 'c1', name: 'Mine', sourceTokenId: null }]);
	});
});
