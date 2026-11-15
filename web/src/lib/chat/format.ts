// Turns a server `RollResult` into what a roll card shows. The UI never
// computes a roll or a total (ADR 0006): everything here is the server's data,
// laid out for reading.
import { RollMode, RollVisibility, type RollResult } from '@tomeandtile/contracts';

/** `+3` or `-2`, always with a sign. */
export function signed(n: number): string {
	return n < 0 ? `-${Math.abs(n)}` : `+${n}`;
}

export interface DieView {
	/** For example `d20: 17` or `d20: 3 (dropped)`. */
	text: string;
	dropped: boolean;
}

export interface ModifierView {
	/** Where the number comes from, for example `Strength`. Empty if unnamed. */
	source: string;
	/** The signed number, for example `+3`. */
	value: string;
}

export interface RollCardView {
	/** The label if there is one, otherwise the notation. */
	title: string;
	/** The notation, shown under the title when there is a label. */
	notation: string;
	/** `Advantage`, `Disadvantage` or empty. */
	modeText: string;
	dice: DieView[];
	modifiers: ModifierView[];
	total: number;
	critical: boolean;
	fumble: boolean;
	/** True for a GM-only roll: only the GM sees it. */
	hidden: boolean;
}

export function modeText(mode: RollMode): string {
	if (mode === RollMode.ADVANTAGE) return 'Advantage';
	if (mode === RollMode.DISADVANTAGE) return 'Disadvantage';
	return '';
}

export function rollCardView(roll: RollResult): RollCardView {
	const label = roll.label.trim();
	return {
		title: label || roll.notation,
		notation: label ? roll.notation : '',
		modeText: modeText(roll.mode),
		dice: roll.dice.map((die) => ({
			text: `d${die.sides}: ${die.value}${die.dropped ? ' (dropped)' : ''}`,
			dropped: die.dropped
		})),
		modifiers: roll.modifiers.map((m) => ({ source: m.source, value: signed(m.value) })),
		total: roll.total,
		critical: roll.critical,
		fumble: roll.fumble,
		hidden: roll.visibility === RollVisibility.GM_ONLY
	};
}

/** One sentence for screen readers, for example `Pip rolled Stealth: 17, critical`. */
export function rollAnnouncement(who: string, view: RollCardView): string {
	const flags = [view.critical ? 'critical' : '', view.fumble ? 'fumble' : '']
		.filter(Boolean)
		.join(', ');
	return `${who} rolled ${view.title}: ${view.total}${flags ? `, ${flags}` : ''}`;
}
