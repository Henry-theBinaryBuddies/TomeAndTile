// What the character sheet shows, built from the server's `Creature`. The UI never
// computes a modifier, a total or a bonus (ADR 0005, ADR 0006): every number here is
// exactly what the server sent, laid out for reading. Where the contract cannot say a
// value was left out (a plain number has no "unset"), zero is shown as unknown only for
// values that can never really be zero: armor class, maximum hit points, proficiency
// bonus and ability scores.
import {
	AbilityScore,
	ActionCost,
	DamageType,
	Role,
	TargetShape,
	type Ability,
	type Creature,
	type EffectStep,
	type Token
} from '@tomeandtile/contracts';

const UNKNOWN = 'Unknown';

export function signed(n: number): string {
	return n < 0 ? `-${Math.abs(n)}` : `+${n}`;
}

function title(word: string): string {
	return word.charAt(0).toUpperCase() + word.slice(1).toLowerCase();
}

function damageTypeWord(type: DamageType): string {
	return type === DamageType.UNSPECIFIED ? '' : (DamageType[type] ?? '').toLowerCase();
}

function abilityScoreWord(score: AbilityScore): string {
	return score === AbilityScore.UNSPECIFIED ? UNKNOWN : title(AbilityScore[score] ?? UNKNOWN);
}

// ---- who the sheet belongs to ----

export interface SheetChoice {
	creatureId: string;
	name: string;
	/** The token that acts for this creature, or null if it has none on the map yet. */
	sourceTokenId: string | null;
}

/**
 * The creatures this viewer can use: the ones they own, the ones with a token they control,
 * and for the GM every creature the server sent. Which sheets reach the viewer at all is the
 * server's decision; this only picks among them.
 */
export function sheetChoices(state: {
	userId: string;
	role: Role;
	creatures: Readonly<Record<string, Creature>>;
	tokens: Readonly<Record<string, Token>>;
}): SheetChoice[] {
	const tokens = Object.values(state.tokens);
	const choices: SheetChoice[] = [];
	for (const creature of Object.values(state.creatures)) {
		const linked = tokens.filter((t) => t.creatureId === creature.creatureId);
		const controlled = linked.filter((t) => t.controllerUserIds.includes(state.userId));
		const mine =
			(state.userId !== '' && creature.ownerUserId === state.userId) ||
			controlled.length > 0 ||
			state.role === Role.GM;
		if (!mine) continue;
		choices.push({
			creatureId: creature.creatureId,
			name: creature.name.trim() || UNKNOWN,
			sourceTokenId: (controlled[0] ?? linked[0])?.tokenId ?? null
		});
	}
	return choices;
}

// ---- the stat block ----

export interface Labelled {
	label: string;
	value: string;
}

export interface StatBlockView {
	name: string;
	summary: string;
	stats: Labelled[];
	/** Null when the server did not send ability scores. */
	scores: Labelled[] | null;
	lists: Labelled[];
	resources: Labelled[];
}

function score(n: number): string {
	return n > 0 ? String(n) : UNKNOWN;
}

function words(list: readonly DamageType[]): string {
	const text = list.map(damageTypeWord).filter(Boolean).join(', ');
	return text;
}

export function statBlockView(c: Creature): StatBlockView {
	const hp =
		c.hpMax > 0
			? `${c.hpCurrent} of ${c.hpMax}${c.hpTemp > 0 ? ` (+${c.hpTemp} temporary)` : ''}`
			: UNKNOWN;
	const stats: Labelled[] = [
		{ label: 'Hit points', value: hp },
		{ label: 'Armor class', value: c.armorClass > 0 ? String(c.armorClass) : UNKNOWN },
		{ label: 'Speed', value: `${c.speedFeet} ft` },
		{ label: 'Initiative', value: signed(c.initiativeBonus) },
		{ label: 'Proficiency', value: c.proficiencyBonus > 0 ? signed(c.proficiencyBonus) : UNKNOWN }
	];
	const lists: Labelled[] = [
		{ label: 'Saving throws', value: c.saveProficiencies.map(abilityScoreWord).join(', ') },
		{ label: 'Skills', value: c.skills.map((s) => `${title(s.skill.replaceAll('_', ' '))} ${signed(s.bonus)}`).join(', ') },
		{ label: 'Resistances', value: words(c.resistances) },
		{ label: 'Immunities', value: words(c.immunities) },
		{ label: 'Vulnerabilities', value: words(c.vulnerabilities) },
		{ label: 'Conditions', value: c.conditions.join(', ') }
	].filter((l) => l.value !== '');
	return {
		name: c.name.trim() || UNKNOWN,
		summary: c.summary.trim(),
		stats,
		scores: c.scores
			? [
					{ label: 'STR', value: score(c.scores.strength) },
					{ label: 'DEX', value: score(c.scores.dexterity) },
					{ label: 'CON', value: score(c.scores.constitution) },
					{ label: 'INT', value: score(c.scores.intelligence) },
					{ label: 'WIS', value: score(c.scores.wisdom) },
					{ label: 'CHA', value: score(c.scores.charisma) }
				]
			: null,
		lists,
		resources: c.resources.map((r) => ({
			label: r.label.trim() || UNKNOWN,
			value: `${r.current} of ${r.max}`
		}))
	};
}

// ---- abilities ----

export type AbilityKind = 'usable' | 'passive' | 'no_steps' | 'area' | 'unsupported';

export interface AbilityView {
	abilityId: string;
	name: string;
	description: string;
	cost: ActionCost;
	kind: AbilityKind;
	/** Why it cannot be clicked, when it cannot. */
	note: string;
	rangeText: string;
	/** For example "Uses 1 Second Wind (2 of 2 left)". Empty if it uses nothing. */
	resourceText: string;
	/** One line per step: to-hit and damage as the server sent them. */
	stepLines: string[];
	shape: TargetShape;
	/** Number of attack steps, which decides how many targets are picked in order. */
	attackSteps: number;
	/** The most targets the ability normally has; 0 if the server did not say. */
	maxTargets: number;
	/** True while a recharge ability is spent. */
	recharging: boolean;
}

const AREA_SHAPES = [
	TargetShape.SPHERE,
	TargetShape.CONE,
	TargetShape.LINE,
	TargetShape.CUBE
] as const;

function damageText(parts: readonly { dice: string; type: DamageType }[]): string {
	return parts
		.map((p) => [p.dice, damageTypeWord(p.type)].filter(Boolean).join(' '))
		.filter(Boolean)
		.join(' + ');
}

function stepLine(step: EffectStep): string {
	const s = step.step;
	switch (s.case) {
		case 'attack': {
			const dmg = damageText(s.value.damage);
			return `Attack ${signed(s.value.toHitBonus)} to hit${dmg ? `, damage ${dmg}` : ''}`;
		}
		case 'save': {
			const dmg = damageText(s.value.damage);
			return (
				`${abilityScoreWord(s.value.ability)} save, DC ${s.value.dc}` +
				(dmg ? `, damage ${dmg}` : '') +
				(s.value.halfOnSuccess ? ', half on a success' : '')
			);
		}
		case 'flat': {
			const dmg = damageText(s.value.damage);
			const heal = s.value.healingDice ? `heals ${s.value.healingDice}` : '';
			return [dmg ? `Damage ${dmg}` : '', heal ? title(heal) : ''].filter(Boolean).join(', ') || 'Effect';
		}
		case 'applyEffect':
			return `Applies ${s.value.effectId || 'an effect'}`;
		case 'plugin':
			return 'Special effect';
		default:
			return 'Effect';
	}
}

function rangeText(a: Ability): string {
	const t = a.targeting;
	if (!t) return UNKNOWN;
	switch (t.shape) {
		case TargetShape.SELF:
			return 'Self';
		case TargetShape.CREATURES:
			if (t.rangeFeet <= 0) return 'Touch';
			return t.longRangeFeet > 0 ? `${t.rangeFeet}/${t.longRangeFeet} ft` : `${t.rangeFeet} ft`;
		case TargetShape.UNSPECIFIED:
			return UNKNOWN;
		default:
			return t.sizeFeet > 0 ? `Area, ${t.sizeFeet} ft` : 'Area';
	}
}

function resourceText(a: Ability, creature: Creature): string {
	if (a.resourceKey === '') return '';
	const resource = creature.resources.find((r) => r.key === a.resourceKey);
	const uses = a.resourceAmount > 0 ? `Uses ${a.resourceAmount}` : 'Uses';
	if (!resource) return `${uses} (amount left unknown)`;
	const label = resource.label.trim() || 'a resource';
	return `${uses} ${label} (${resource.current} of ${resource.max} left)`;
}

export function abilityView(a: Ability, creature: Creature): AbilityView {
	const shape = a.targeting?.shape ?? TargetShape.UNSPECIFIED;
	const attackSteps = a.steps.filter((s) => s.step.case === 'attack').length;
	let kind: AbilityKind = 'usable';
	let note = '';
	if (a.cost === ActionCost.PASSIVE) {
		kind = 'passive';
	} else if (a.steps.length === 0) {
		kind = 'no_steps';
	} else if ((AREA_SHAPES as readonly TargetShape[]).includes(shape)) {
		kind = 'area';
		note = 'Area abilities come later.';
	} else if (shape === TargetShape.UNSPECIFIED) {
		kind = 'unsupported';
		note = 'This ability has no target type yet.';
	}
	return {
		abilityId: a.abilityId,
		name: a.name.trim() || UNKNOWN,
		description: a.description.trim(),
		cost: a.cost,
		kind,
		note,
		rangeText: rangeText(a),
		resourceText: resourceText(a, creature),
		stepLines: a.steps.map(stepLine),
		shape,
		attackSteps,
		maxTargets: a.targeting?.maxTargets ?? 0,
		recharging: a.rechargeSpent
	};
}

export interface AbilityGroup {
	cost: ActionCost;
	label: string;
	abilities: AbilityView[];
}

const GROUPS: [ActionCost, string][] = [
	[ActionCost.ACTION, 'Actions'],
	[ActionCost.BONUS_ACTION, 'Bonus actions'],
	[ActionCost.REACTION, 'Reactions'],
	[ActionCost.FREE, 'Free'],
	[ActionCost.LEGENDARY, 'Legendary actions'],
	[ActionCost.PASSIVE, 'Passive'],
	[ActionCost.UNSPECIFIED, 'Other']
];

/** Abilities grouped by action cost, in the server's order within each group. Empty groups are left out. */
export function abilityGroups(creature: Creature): AbilityGroup[] {
	return GROUPS.map(([cost, label]) => ({
		cost,
		label,
		abilities: creature.abilities.filter((a) => a.cost === cost).map((a) => abilityView(a, creature))
	})).filter((g) => g.abilities.length > 0);
}
