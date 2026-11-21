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
		limit: ordered ? ability.attackSteps : ability.maxTargets
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

/** A note, never a block, when more targets are picked than the ability normally has. */
export function limitWarning(s: TargetingState): string {
	if (s.limit <= 0 || s.picks.length <= s.limit) return '';
	const what = s.ordered
		? `${s.abilityName} has ${s.limit} attacks`
		: `${s.abilityName} normally has ${s.limit} ${s.limit === 1 ? 'target' : 'targets'}`;
	return `${what}, and you picked ${s.picks.length}. You can still confirm; the server decides what happens.`;
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
