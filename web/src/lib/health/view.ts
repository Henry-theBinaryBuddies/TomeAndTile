// How hurt a creature is, in words. Players see only what the server sends: every token
// carries a `health_state`, and the HP numbers come only to the GM and the token's controllers
// (ADR 0006). The UI never decides who sees numbers and never derives a state from HP: the
// thresholds are the server's (scene.proto), so a state it did not send reads "Health unknown",
// never "Healthy".
import { HealthState, type Token } from '@tomeandtile/contracts';

/** The parts of a token that say how hurt it is. Anything with these fields will do. */
export type HealthInfo = Partial<Pick<Token, 'healthState' | 'hpCurrent' | 'hpMax' | 'hpTemp'>>;

export interface HealthView {
	/** Healthy, Bloodied, Near death, Down, or "Health unknown". */
	stateText: string;
	/** Down is also shown with a mark and a border, so it never relies on colour alone. */
	down: boolean;
	/** "9 of 13", only when the server sent both numbers; null otherwise (never "0"). */
	hp: string | null;
	/** "+3 temporary" when there are temporary hit points, else null. */
	temp: string | null;
	/** One short line for a row: "9 of 13, Bloodied", or just the state. */
	short: string;
	/** One sentence for screen readers. */
	label: string;
}

const STATE_TEXT: Partial<Record<HealthState, string>> = {
	[HealthState.HEALTHY]: 'Healthy',
	[HealthState.BLOODIED]: 'Bloodied',
	[HealthState.NEAR_DEATH]: 'Near death',
	[HealthState.DOWN]: 'Down'
};

export const HEALTH_UNKNOWN = 'Health unknown';

export function healthStateText(state: HealthState | undefined): string {
	return (state !== undefined && STATE_TEXT[state]) || HEALTH_UNKNOWN;
}

/** `token` is undefined for a combatant or creature with no token the viewer can see. */
export function healthView(token: HealthInfo | undefined): HealthView {
	const stateText = healthStateText(token?.healthState);
	const hasNumbers =
		token?.hpCurrent !== undefined && token.hpMax !== undefined && token.hpMax > 0;
	const hp = hasNumbers ? `${token.hpCurrent} of ${token.hpMax}` : null;
	const temp = hasNumbers && token.hpTemp !== undefined && token.hpTemp > 0 ? `+${token.hpTemp} temporary` : null;
	const numbers = hp ? `${hp}${temp ? ` (${temp})` : ''}` : null;
	return {
		stateText,
		down: token?.healthState === HealthState.DOWN,
		hp,
		temp,
		short: numbers ? `${numbers}, ${stateText}` : stateText,
		label: numbers ? `${numbers} hit points, ${stateText}` : stateText
	};
}
