import { describe, expect, it } from 'vitest';
import { ActionCost, HealthState, Role, TargetShape, type Token } from '@tomeandtile/contracts';
import { applySnapshot } from '../net/state';
import { CHARACTER_FILES, loadCharacter, loadPregenTable } from './fixtures';
import { abilityGroups, abilityView, actingTokenId, sheetChoices, signed, statBlockView } from './view';

// Every case starts from a parsed fixture (contracts/fixtures) and changes only the field it
// is about, so the tests use the same shapes the server sends.
const brannoch = () => loadCharacter('brannoch-vale-fighter');
const ilvaren = () => loadCharacter('ilvaren-quillfeather-wizard');
const ability = (c: ReturnType<typeof brannoch>, id: string) =>
	abilityView(c.abilities.find((a) => a.abilityId === id)!, c);
const stat = (c: ReturnType<typeof brannoch>, label: string) =>
	statBlockView(c).stats.find((x) => x.label === label)?.value;

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

	it('shows passive Perception as sent, and unknown when it was not stated', () => {
		expect(stat(brannoch(), 'Passive Perception')).toBe('13');
		const c = brannoch();
		c.passivePerception = 0;
		expect(stat(c, 'Passive Perception')).toBe('Unknown');
	});

	it('shows temporary hit points when there are some', () => {
		const c = brannoch();
		c.hpTemp = 3;
		expect(stat(c, 'Hit points')).toBe('28 of 28 (+3 temporary)');
	});

	it('shows unknown, never zero, for each value the server did not send', () => {
		const hp = brannoch();
		hp.hpMax = 0;
		expect(stat(hp, 'Hit points')).toBe('Unknown');
		const ac = brannoch();
		ac.armorClass = 0;
		expect(stat(ac, 'Armor class')).toBe('Unknown');
		const prof = brannoch();
		prof.proficiencyBonus = 0;
		expect(stat(prof, 'Proficiency')).toBe('Unknown');
		const name = brannoch();
		name.name = '  ';
		expect(statBlockView(name).name).toBe('Unknown');
		const none = brannoch();
		none.scores = undefined;
		expect(statBlockView(none).scores).toBeNull();
	});

	it('shows an ability score of zero as unknown', () => {
		const c = brannoch();
		c.scores!.dexterity = 0;
		const scores = statBlockView(c).scores!;
		expect(scores[0].value).toBe('17');
		expect(scores[1].value).toBe('Unknown');
	});

	it('lists resources as current of max', () => {
		expect(statBlockView(brannoch()).resources).toContainEqual({ label: 'Second Wind', value: '2 of 2' });
	});

	it('keeps text from the server as plain text', () => {
		const c = brannoch();
		c.name = '<img src=x onerror=alert(1)>';
		c.summary = '<b>hi</b>';
		const b = statBlockView(c);
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

	it('show how many targets an ability takes', () => {
		const c = brannoch();
		expect(ability(c, 'longsword').targetText).toBe('Up to 1 target');
		expect(ability(c, 'second-wind').targetText).toBe('');
		const sword = c.abilities.find((a) => a.abilityId === 'longsword')!;
		sword.targeting!.maxTargets = 3;
		expect(abilityView(sword, c).targetText).toBe('Up to 3 targets');
		sword.targeting!.maxTargets = 0;
		expect(abilityView(sword, c).targetText).toBe('');
	});

	it('show a multi-attack as one target per attack', () => {
		const c = brannoch();
		const sword = c.abilities.find((a) => a.abilityId === 'longsword')!;
		sword.steps = [sword.steps[0], sword.steps[0]];
		const v = abilityView(sword, c);
		expect(v.attackSteps).toBe(2);
		expect(v.targetText).toBe('2 attacks, one target each');
	});

	it('show the resource and what is left', () => {
		const c = brannoch();
		expect(ability(c, 'second-wind').resourceText).toBe('Uses 1 Second Wind (2 of 2 left)');
		expect(ability(c, 'second-wind').rangeText).toBe('Self');
		expect(ability(c, 'longsword').resourceText).toBe('');
	});

	it('say the amount left is unknown when the named resource is not on the sheet', () => {
		const c = brannoch();
		c.resources = c.resources.filter((r) => r.key !== 'second_wind');
		expect(c.abilities.find((a) => a.abilityId === 'second-wind')!.resourceKey).not.toBe('');
		expect(ability(c, 'second-wind').resourceText).toMatch(/^Uses 1 \(amount left unknown\)$/);
	});

	it('treat a passive ability and one with no steps as plain text, not clickable', () => {
		const c = brannoch();
		c.abilities.find((a) => a.abilityId === 'longsword')!.cost = ActionCost.PASSIVE;
		c.abilities.find((a) => a.abilityId === 'longbow')!.steps = [];
		expect(ability(c, 'longsword').kind).toBe('passive');
		expect(ability(c, 'longbow').kind).toBe('no_steps');
	});

	it('show an area ability disabled, with a note that areas come later', () => {
		const c = ilvaren();
		const v = ability(c, 'burning-hands');
		expect(v.kind).toBe('area');
		expect(v.note).toBe('Area abilities come later.');
		expect(v.rangeText).toBe('15 ft cone');
		expect(v.stepLines[0]).toContain('3d6 fire');
	});

	it('include the range of an area ability when the server sends one', () => {
		const c = ilvaren();
		const spell = c.abilities.find((a) => a.abilityId === 'burning-hands')!;
		spell.targeting!.shape = TargetShape.SPHERE;
		spell.targeting!.rangeFeet = 150;
		spell.targeting!.sizeFeet = 20;
		expect(abilityView(spell, c).rangeText).toBe('150 ft, 20 ft sphere');
		spell.targeting!.sizeFeet = 0;
		expect(abilityView(spell, c).rangeText).toBe('150 ft, sphere');
	});
});

describe('sheetChoices', () => {
	const table = () => applySnapshot(loadPregenTable());
	const asPlayer = () => ({ ...table(), userId: 'mock-player', role: Role.PLAYER });
	const copyOf = (t: Token, over: Partial<Token>): Token => ({ ...t, ...over });

	it('gives mock-player all five pregens, each with its token', () => {
		const choices = sheetChoices(asPlayer());
		expect(choices).toHaveLength(5);
		expect(choices.map((c) => c.name)).toContain('Brannoch Vale');
		const b = choices.find((c) => c.creatureId === 'creature-brannoch')!;
		expect(b.sourceTokenId).toBe('token-brannoch');
		expect(b.tokens).toEqual([{ tokenId: 'token-brannoch', name: expect.any(String) }]);
	});

	it('gives a player nothing they neither own nor control', () => {
		expect(sheetChoices({ ...table(), userId: 'someone-else', role: Role.PLAYER })).toEqual([]);
	});

	it('gives the GM every creature the server sent', () => {
		expect(sheetChoices({ ...table(), userId: 'mock-gm', role: Role.GM }).length).toBeGreaterThanOrEqual(6);
	});

	it('includes an owned creature that has no token yet, with no source token', () => {
		const state = asPlayer();
		const mine = state.creatures['creature-brannoch'];
		mine.ownerUserId = 'mock-player';
		const choice = sheetChoices({ ...state, tokens: {} }).find((c) => c.creatureId === mine.creatureId)!;
		expect(choice.tokens).toEqual([]);
		expect(choice.sourceTokenId).toBeNull();
	});

	it('uses only tokens on the active map', () => {
		const state = asPlayer();
		const original = state.tokens['token-brannoch'];
		const elsewhere = copyOf(original, { tokenId: 'token-brannoch-elsewhere', sceneId: 'scene-other' });
		const tokens = { ...state.tokens, [elsewhere.tokenId]: elsewhere };
		const choice = sheetChoices({ ...state, tokens }).find((c) => c.creatureId === 'creature-brannoch')!;
		expect(choice.tokens.map((t) => t.tokenId)).toEqual(['token-brannoch']);
		expect(choice.sourceTokenId).toBe('token-brannoch');
	});

	it('has no acting token when the only one is on another map', () => {
		const state = asPlayer();
		const moved = copyOf(state.tokens['token-brannoch'], { sceneId: 'scene-other' });
		const choice = sheetChoices({ ...state, tokens: { ...state.tokens, 'token-brannoch': moved } }).find(
			(c) => c.creatureId === 'creature-brannoch'
		)!;
		expect(choice.tokens).toEqual([]);
		expect(choice.sourceTokenId).toBeNull();
	});

	it('prefers the tokens the viewer controls over ones that are only linked', () => {
		const state = asPlayer();
		const linkedOnly = copyOf(state.tokens['token-brannoch'], { tokenId: 'token-brannoch-npc', controllerUserIds: [] });
		const tokens = { ...state.tokens, [linkedOnly.tokenId]: linkedOnly };
		const choice = sheetChoices({ ...state, tokens }).find((c) => c.creatureId === 'creature-brannoch')!;
		expect(choice.tokens.map((t) => t.tokenId)).toEqual(['token-brannoch']);
	});

	it('does not pick one when several tokens could act; the person chooses', () => {
		const state = asPlayer();
		const second = copyOf(state.tokens['token-brannoch'], { tokenId: 'token-brannoch-2', name: 'Brannoch (2)' });
		const tokens = { ...state.tokens, [second.tokenId]: second };
		const choice = sheetChoices({ ...state, tokens }).find((c) => c.creatureId === 'creature-brannoch')!;
		expect(choice.tokens).toHaveLength(2);
		expect(choice.sourceTokenId).toBeNull();
		expect(actingTokenId(choice, '')).toBeNull();
		expect(actingTokenId(choice, 'token-brannoch-2')).toBe('token-brannoch-2');
		expect(actingTokenId(choice, 'not-a-token')).toBeNull();
	});

	it('acts with the only token without being asked', () => {
		const choice = sheetChoices(asPlayer()).find((c) => c.creatureId === 'creature-brannoch')!;
		expect(actingTokenId(choice, '')).toBe('token-brannoch');
	});
});

describe('statBlockView hit points and health from the token', () => {
	const stat = (b: ReturnType<typeof statBlockView>, label: string) =>
		b.stats.find((x) => x.label === label)?.value;

	it('shows the token numbers and state, and the token wins over the creature', () => {
		const c = brannoch();
		const t = { healthState: HealthState.BLOODIED, hpCurrent: 3, hpMax: 20, hpTemp: 2 };
		const b = statBlockView(c, t);
		expect(stat(b, 'Hit points')).toBe('3 of 20 (+2 temporary)');
		expect(stat(b, 'Health')).toBe('Bloodied');
	});

	it('shows Unknown, not 0, when the token has a state but no numbers', () => {
		const b = statBlockView(brannoch(), { healthState: HealthState.HEALTHY });
		expect(stat(b, 'Hit points')).toBe('Unknown');
		expect(stat(b, 'Health')).toBe('Healthy');
	});

	it('shows the creature numbers and unknown health when there is no token', () => {
		const b = statBlockView(brannoch());
		expect(stat(b, 'Hit points')).not.toBe('Unknown');
		expect(stat(b, 'Health')).toBe('Health unknown');
	});
});
