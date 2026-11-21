// Turns a server `Resolution` into what an ability result card shows. The UI never
// computes a roll, a total, a hit or damage (ADR 0005, ADR 0006): every number and every
// outcome here is the server's, laid out for reading. Anything missing shows as unknown.
import {
	HitOutcome,
	ResolutionState,
	type Creature,
	type Resolution,
	type TargetOutcome,
	type Token,
	type Warning
} from '@tomeandtile/contracts';
import { rollCardView, type RollCardView } from './format';

type Tokens = Readonly<Record<string, Pick<Token, 'name' | 'creatureId'>>>;
type Creatures = Readonly<Record<string, Pick<Creature, 'abilities'>>>;

const WARNING_TEXT: Record<string, string> = {
	out_of_range: 'A target was out of range. The attack was still rolled.',
	too_many_targets: 'More targets than this ability allows. All of them were still used.',
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
			return { text: 'Save failed', kind: 'other' };
		case HitOutcome.SAVE_SUCCEEDED:
			return { text: 'Save succeeded', kind: 'other' };
		case HitOutcome.AUTOMATIC:
			return { text: 'No roll needed', kind: 'other' };
		default:
			return { text: 'Unknown', kind: 'other' };
	}
}

export interface TargetView {
	/** Position in the card, used as the list key (the same token can appear more than once). */
	key: number;
	name: string;
	/** `1`, `2`, ... for the attack that produced this outcome; null unless it is a multi-attack. */
	attackNumber: number | null;
	outcome: OutcomeView;
	/** The attack roll or saving throw, or null if there is none to show. */
	roll: RollCardView | null;
	/** Said instead of a roll: "Roll: unknown" when it is missing, or nothing when none was needed. */
	rollNote: string;
	damageRolls: RollCardView[];
}

export interface ResultCardView {
	abilityName: string;
	sourceName: string;
	/** Empty when done; otherwise "In progress" or "Cancelled". */
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

function targetView(
	outcome: TargetOutcome,
	index: number,
	tokens: Tokens,
	multi: boolean
): TargetView {
	const view = outcomeView(outcome.outcome);
	const noRollNeeded = outcome.outcome === HitOutcome.AUTOMATIC;
	return {
		key: index,
		name: nameOf(tokens, outcome.tokenId),
		attackNumber: multi ? outcome.stepIndex + 1 : null,
		outcome: view,
		roll: outcome.roll ? rollCardView(outcome.roll) : null,
		rollNote: outcome.roll || noRollNeeded ? '' : 'Roll: unknown',
		damageRolls: outcome.damageRolls.map(rollCardView)
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
	const targets = resolution.outcomes.map((o, i) => targetView(o, i, tokens, multi));
	const results = targets
		.map((t) => `${t.attackNumber ? `attack ${t.attackNumber} ` : ''}${t.name}: ${t.outcome.text}`)
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
