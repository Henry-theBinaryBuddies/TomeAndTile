// Choosing targets for an ability, and the intent that comes out of it. Pure logic: it holds
// the picks and builds the request. Range, target counts and permissions are the server's
// business (ADR 0005): going over a limit only shows a note, and nothing here ever blocks a
// confirm for those reasons.
import { create } from '@bufbuild/protobuf';
import { RollMode, TargetShape, UseAbilityIntentSchema } from '@tomeandtile/contracts';
import type { IntentKind } from '../net';
import type { AbilityView } from './view';

export interface TargetingState {
	abilityId: string;
	abilityName: string;
	shape: TargetShape;
	/** Token ids in the order they were picked. */
	picks: string[];
	mode: RollMode;
	/**
	 * Several attack steps: one pick per step, in order, and the same token may be picked again.
	 * Otherwise a click picks a token or takes the pick back.
	 */
	ordered: boolean;
	/** How many picks the ability normally takes. 0 if unknown. */
	limit: number;
	/** The same count in words, for the hint ("Up to 2 targets"). Empty if not stated. */
	targetText: string;
}

export function startTargeting(ability: AbilityView): TargetingState {
	const ordered = ability.attackSteps > 1;
	return {
		abilityId: ability.abilityId,
		abilityName: ability.name,
		shape: ability.shape,
		picks: [],
		mode: RollMode.NORMAL,
		ordered,
		limit: ordered ? ability.attackSteps : ability.maxTargets,
		targetText: ability.targetText
	};
}

/** True if this ability picks creatures (a self ability has nothing to pick). */
export function picksTargets(s: TargetingState): boolean {
	return s.shape === TargetShape.CREATURES;
}

/** Click on a token: add it (ordered), or add it / take it back (unordered). */
export function pickToken(s: TargetingState, tokenId: string): TargetingState {
	if (!picksTargets(s)) return s;
	if (s.ordered) return { ...s, picks: [...s.picks, tokenId] };
	return s.picks.includes(tokenId)
		? { ...s, picks: s.picks.filter((id) => id !== tokenId) }
		: { ...s, picks: [...s.picks, tokenId] };
}

/** Take back the pick at this position. */
export function removePick(s: TargetingState, index: number): TargetingState {
	return { ...s, picks: s.picks.filter((_, i) => i !== index) };
}

export function setMode(s: TargetingState, mode: RollMode): TargetingState {
	return { ...s, mode };
}

/** How many times this token is picked. */
export function pickCount(s: TargetingState, tokenId: string): number {
	return s.picks.filter((id) => id === tokenId).length;
}

/**
 * A note, never a block, when the picks do not fit the ability. It says what the server will
 * do (ADR 0005): a multi-attack ignores extra picks and does not roll attacks that have no
 * pick (`extra_targets_ignored`); an ability with one attack step still uses every target,
 * with a `too_many_targets` warning. The server decides; this only warns.
 */
export function limitWarning(s: TargetingState): string {
	const picked = s.picks.length;
	const note = 'You can still confirm; the server decides what happens.';
	if (s.ordered) {
		if (picked > 0 && picked < s.limit) {
			const later = s.limit - picked;
			return `${s.abilityName} has ${s.limit} attacks, and you picked ${picked}. The ${later === 1 ? 'last attack' : `last ${later} attacks`} will not be rolled. ${note}`;
		}
		if (picked > s.limit) {
			return `${s.abilityName} has ${s.limit} attacks, and you picked ${picked}. The extra ${picked - s.limit === 1 ? 'pick is' : 'picks are'} ignored. ${note}`;
		}
		return '';
	}
	if (s.limit <= 0 || picked <= s.limit) return '';
	return `${s.abilityName} normally has ${s.limit} ${s.limit === 1 ? 'target' : 'targets'}, and you picked ${picked}. Every target is still used, and the server adds a warning. ${note}`;
}

/** True while the person is still looking at the character the picking started for. */
export function keepTargeting(startedFor: string, shownCreatureId: string): boolean {
	return startedFor === shownCreatureId;
}

/** The request to send. Targets go in pick order. */
export function buildUseAbilityKind(sourceTokenId: string, s: TargetingState): IntentKind {
	return {
		case: 'useAbility',
		value: create(UseAbilityIntentSchema, {
			sourceTokenId,
			abilityId: s.abilityId,
			targetTokenIds: picksTargets(s) ? s.picks : [],
			mode: s.mode
		})
	};
}
