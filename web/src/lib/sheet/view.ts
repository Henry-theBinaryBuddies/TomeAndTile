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
import { healthView, type HealthInfo } from '../health';

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

export interface ActingToken {
	tokenId: string;
	name: string;
}

export interface SheetChoice {
	creatureId: string;
	name: string;
	/**
	 * The tokens on the active map that could act for this creature: the ones the viewer
	 * controls if there are any, otherwise the ones linked to it. Usually one.
	 */
	tokens: ActingToken[];
	/** The token that acts for this creature when there is exactly one; null if none or several. */
	sourceTokenId: string | null;
}

/** The token that acts: the only one, or the one the person chose from several. Never a guess. */
export function actingTokenId(choice: SheetChoice, chosenTokenId: string): string | null {
	if (choice.tokens.length === 1) return choice.tokens[0].tokenId;
	return choice.tokens.find((t) => t.tokenId === chosenTokenId)?.tokenId ?? null;
}

/**
 * The creatures this viewer can use: the ones they own, the ones with a token they control,
 * and for the GM every creature the server sent. Which sheets reach the viewer at all is the
 * server's decision; this only picks among them.
 */
export function sheetChoices(state: {
	userId: string;
	role: Role;
	/** The scene on the map now; '' if unknown, which counts every token. */
	activeSceneId?: string;
	creatures: Readonly<Record<string, Creature>>;
	tokens: Readonly<Record<string, Token>>;
}): SheetChoice[] {
	const active = state.activeSceneId ?? '';
	const tokens = Object.values(state.tokens);
	const choices: SheetChoice[] = [];
	for (const creature of Object.values(state.creatures)) {
		const allLinked = tokens.filter((t) => t.creatureId === creature.creatureId);
		const controlledAnywhere = allLinked.filter((t) => t.controllerUserIds.includes(state.userId));
		const mine =
			(state.userId !== '' && creature.ownerUserId === state.userId) ||
			controlledAnywhere.length > 0 ||
			state.role === Role.GM;
		if (!mine) continue;
		// Only tokens on the map being shown can act now.
		const onMap = (t: Token) => active === '' || t.sceneId === active;
		const linked = allLinked.filter(onMap);
		const controlled = linked.filter((t) => t.controllerUserIds.includes(state.userId));
		const candidates = controlled.length > 0 ? controlled : linked;
		const acting = candidates.map((t) => ({ tokenId: t.tokenId, name: t.name.trim() || UNKNOWN }));
		choices.push({
			creatureId: creature.creatureId,
			name: creature.name.trim() || UNKNOWN,
			tokens: acting,
			sourceTokenId: acting.length === 1 ? acting[0].tokenId : null
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

/**
 * `token` is the viewer's token for this creature, if there is one. Hit points are live on the
 * token, not on the creature, so when there is a token it decides both the numbers and the
 * health state, even if the creature says something different (scene.proto: several tokens can
 * share one creature). Without a token the creature's own numbers are shown and the health
 * state is unknown.
 */
export function statBlockView(c: Creature, token?: HealthInfo): StatBlockView {
	const health = healthView(token);
	const creatureHp =
		c.hpMax > 0
			? `${c.hpCurrent} of ${c.hpMax}${c.hpTemp > 0 ? ` (+${c.hpTemp} temporary)` : ''}`
			: UNKNOWN;
	const tokenHp = health.hp ? `${health.hp}${health.temp ? ` (${health.temp})` : ''}` : null;
	const hp = token ? (tokenHp ?? UNKNOWN) : creatureHp;
	const stats: Labelled[] = [
		{ label: 'Hit points', value: hp },
		{ label: 'Health', value: health.stateText },
		{ label: 'Armor class', value: c.armorClass > 0 ? String(c.armorClass) : UNKNOWN },
		{ label: 'Speed', value: `${c.speedFeet} ft` },
		{ label: 'Initiative', value: signed(c.initiativeBonus) },
		{ label: 'Proficiency', value: c.proficiencyBonus > 0 ? signed(c.proficiencyBonus) : UNKNOWN },
		// A plain number cannot say "not stated", and a real passive Perception is never 0.
		{ label: 'Passive Perception', value: c.passivePerception > 0 ? String(c.passivePerception) : UNKNOWN }
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
	/** For example "Up to 2 targets" or "2 attacks, one target each". Empty if not stated. */
	targetText: string;
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
		default: {
			const shape = (TargetShape[t.shape] ?? 'area').toLowerCase();
			const size = t.sizeFeet > 0 ? `${t.sizeFeet} ft ${shape}` : shape;
			return t.rangeFeet > 0 ? `${t.rangeFeet} ft, ${size}` : size;
		}
	}
}

/** How many targets the ability takes, in words. Empty when the server did not say. */
function targetText(shape: TargetShape, attackSteps: number, maxTargets: number): string {
	if (shape !== TargetShape.CREATURES) return '';
	if (attackSteps > 1) return `${attackSteps} attacks, one target each`;
	if (maxTargets > 0) return `Up to ${maxTargets} ${maxTargets === 1 ? 'target' : 'targets'}`;
	return '';
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
		targetText: targetText(shape, attackSteps, a.targeting?.maxTargets ?? 0),
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
