// Turns a server `Resolution` into what an ability result card shows. The UI never
// computes a roll, a total, a hit or damage (ADR 0005, ADR 0006): every number and every
// outcome here is the server's, laid out for reading. Anything missing shows as unknown.
import {
	DamageType,
	HitOutcome,
	ResolutionState,
	RollPurpose,
	type Creature,
	type DamageApplied,
	type Resolution,
	type TargetOutcome,
	type Token,
	type Warning
} from '@tomeandtile/contracts';
import { conditionName } from '../conditions';
import { rollCardView, type RollCardView } from './format';

type Tokens = Readonly<Record<string, Pick<Token, 'name' | 'creatureId'>>>;
type Creatures = Readonly<Record<string, Pick<Creature, 'abilities'>>>;

const WARNING_TEXT: Record<string, string> = {
	out_of_range: 'A target was out of range. The attack was still rolled.',
	too_many_targets: 'More targets than this ability allows. All of them were still used.',
	condition_immune: 'That creature is immune to that condition. It was added anyway.',
	not_enough_resource: 'Not enough of what this ability uses. It was still used.',
	extra_targets_ignored: 'Some of the targets were ignored because there were not enough attacks.'
};

/** Plain words for a warning code. The server's own message is never shown. */
export function warningText(code: string): string {
	return WARNING_TEXT[code] ?? `The server added a note about this (${code || 'no reason given'}).`;
}

export type OutcomeKind = 'hit' | 'critical' | 'miss' | 'other';

export interface OutcomeView {
	text: string;
	kind: OutcomeKind;
}

export function outcomeView(outcome: HitOutcome): OutcomeView {
	switch (outcome) {
		case HitOutcome.HIT:
			return { text: 'Hit', kind: 'hit' };
		case HitOutcome.CRITICAL_HIT:
			return { text: 'Critical hit', kind: 'critical' };
		case HitOutcome.MISS:
			return { text: 'Miss', kind: 'miss' };
		case HitOutcome.SAVE_FAILED:
			return { text: 'Failed the save', kind: 'other' };
		case HitOutcome.SAVE_SUCCEEDED:
			return { text: 'Saved', kind: 'other' };
		case HitOutcome.AUTOMATIC:
			return { text: 'Automatic', kind: 'other' };
		default:
			return { text: 'Unknown', kind: 'other' };
	}
}

/**
 * One damage entry, as the server sent it: "7 fire". The applied figure is the server's, after
 * resistance, immunity, vulnerability and half on a save. When it differs from the rolled figure
 * the rolled one is shown too. The card never says why: the contract does not.
 */
export function damageText(d: DamageApplied): string {
	const kind = d.type === DamageType.UNSPECIFIED ? 'damage' : (DamageType[d.type] ?? 'damage').toLowerCase();
	return d.applied === d.rolled
		? `${d.applied} ${kind}`
		: `${d.applied} ${kind} (rolled ${d.rolled})`;
}

export interface TargetView {
	/** Position in the card, used as the list key (the same token can appear more than once). */
	key: number;
	name: string;
	/** The target's token id, for the GM's buttons on this row. */
	tokenId: string;
	/** Total damage the server applied to this row, or null when it sent no damage list. */
	appliedTotal: number | null;
	/** `1`, `2`, ... for the attack that produced this outcome; null unless it is a multi-attack. */
	attackNumber: number | null;
	outcome: OutcomeView;
	/** The attack roll or saving throw, or null if there is none to show. */
	roll: RollCardView | null;
	/**
	 * Who rolled `roll`. The attacker rolls an attack; the target rolls its own saving throw.
	 * Damage rolls are always the source's, so they are not covered by this.
	 */
	rollWho: string;
	/** Said instead of a roll: "Roll: unknown" when it is missing, or nothing when none was needed. */
	rollNote: string;
	damageRolls: RollCardView[];
	/** Damage actually applied, one entry per type. Empty when the server sent none: never "0 damage". */
	damage: string[];
	/** "Healed 8" when healing is above 0, else null. */
	healing: string | null;
	/** Conditions and effects applied, by display name. An id the table does not know shows as sent. */
	effects: string[];
	/** True when the server flagged that this dropped the target to 0. Nothing about death saves. */
	droppedToZero: boolean;
}

export interface ResultCardView {
	abilityName: string;
	sourceName: string;
	/** Empty when done; otherwise "In progress", "Cancelled" or, when the server sent no state, "State unknown". */
	stateText: string;
	targets: TargetView[];
	warnings: string[];
	/** One sentence for screen readers. */
	announcement: string;
}

function nameOf(tokens: Tokens, tokenId: string): string {
	return tokens[tokenId]?.name.trim() || 'Unknown';
}

function stateText(state: ResolutionState): string {
	if (state === ResolutionState.DONE) return '';
	if (state === ResolutionState.CANCELLED) return 'Cancelled';
	// An unset state is unknown, not "in progress" (web/CLAUDE.md: unset means unknown).
	if (state === ResolutionState.UNSPECIFIED) return 'State unknown';
	return 'In progress';
}

/** How many attack steps the ability has, if the viewer can see the creature's sheet. */
function attackStepCount(
	resolution: Resolution,
	tokens: Tokens,
	creatures: Creatures
): number | null {
	const creatureId = tokens[resolution.sourceTokenId]?.creatureId;
	const creature = creatureId ? creatures[creatureId] : undefined;
	const ability = creature?.abilities.find((a) => a.abilityId === resolution.abilityId);
	if (!ability) return null;
	return ability.steps.filter((s) => s.step.case === 'attack').length;
}

/** A saving throw is rolled by the target, so its roll is not the source's. */
function isSave(outcome: TargetOutcome): boolean {
	return (
		outcome.outcome === HitOutcome.SAVE_FAILED ||
		outcome.outcome === HitOutcome.SAVE_SUCCEEDED ||
		outcome.roll?.purpose === RollPurpose.SAVE
	);
}

function targetView(
	outcome: TargetOutcome,
	index: number,
	tokens: Tokens,
	multi: boolean,
	sourceName: string
): TargetView {
	const view = outcomeView(outcome.outcome);
	const noRollNeeded = outcome.outcome === HitOutcome.AUTOMATIC;
	const name = nameOf(tokens, outcome.tokenId);
	return {
		key: index,
		tokenId: outcome.tokenId,
		appliedTotal: outcome.damage.length > 0 ? outcome.damage.reduce((sum, d) => sum + d.applied, 0) : null,
		name,
		rollWho: isSave(outcome) ? name : sourceName,
		attackNumber: multi ? outcome.stepIndex + 1 : null,
		outcome: view,
		roll: outcome.roll ? rollCardView(outcome.roll) : null,
		rollNote: outcome.roll || noRollNeeded ? '' : 'Roll: unknown',
		damageRolls: outcome.damageRolls.map(rollCardView),
		damage: outcome.damage.map(damageText),
		healing: outcome.healing > 0 ? `Healed ${outcome.healing}` : null,
		effects: outcome.effectsApplied.map((id) => conditionName(id).name),
		droppedToZero: outcome.droppedToZero
	};
}

export function resultCardView(
	resolution: Resolution,
	tokens: Tokens,
	creatures: Creatures,
	warnings: readonly Warning[]
): ResultCardView {
	const abilityName = resolution.abilityName.trim() || 'Unknown ability';
	const sourceName = nameOf(tokens, resolution.sourceTokenId);
	const steps = attackStepCount(resolution, tokens, creatures);
	const distinctSteps = new Set(resolution.outcomes.map((o) => o.stepIndex)).size;
	const multi = (steps !== null && steps > 1) || distinctSteps > 1;
	const targets = resolution.outcomes.map((o, i) => targetView(o, i, tokens, multi, sourceName));
	const results = targets
		.map((t) => {
			const parts = [t.outcome.text, ...t.damage, t.healing?.toLowerCase(), ...t.effects, t.droppedToZero ? 'dropped to 0' : undefined];
			return `${t.attackNumber ? `attack ${t.attackNumber} ` : ''}${t.name}: ${parts.filter(Boolean).join(', ')}`;
		})
		.join('; ');
	return {
		abilityName,
		sourceName,
		stateText: stateText(resolution.state),
		targets,
		warnings: warnings.map((w) => warningText(w.code)),
		announcement: `${sourceName} used ${abilityName}.${results ? ` ${results}.` : ''}`
	};
}
