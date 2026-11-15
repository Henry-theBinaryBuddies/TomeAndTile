import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import {
	DieSchema,
	ModifierSchema,
	RollMode,
	RollResultSchema,
	RollVisibility
} from '@tomeandtile/contracts';
import { modeText, rollAnnouncement, rollCardView, signed } from './format';

const roll = (over: Parameters<typeof create<typeof RollResultSchema>>[1] = {}) =>
	create(RollResultSchema, {
		rollId: 'r1',
		notation: '2d20kh1+5',
		total: 17,
		mode: RollMode.ADVANTAGE,
		dice: [
			create(DieSchema, { sides: 20, value: 12 }),
			create(DieSchema, { sides: 20, value: 4, dropped: true })
		],
		modifiers: [
			create(ModifierSchema, { source: 'Strength', value: 3 }),
			create(ModifierSchema, { source: 'Penalty', value: -1 })
		],
		...over
	});

describe('signed', () => {
	it('always shows a sign', () => {
		expect(signed(3)).toBe('+3');
		expect(signed(0)).toBe('+0');
		expect(signed(-2)).toBe('-2');
	});
});

describe('rollCardView', () => {
	it('lists every die, marking dropped dice in words', () => {
		const v = rollCardView(roll());
		expect(v.dice.map((d) => d.text)).toEqual(['d20: 12', 'd20: 4 (dropped)']);
		expect(v.dice.map((d) => d.dropped)).toEqual([false, true]);
	});

	it('lists modifiers with their source and sign', () => {
		const v = rollCardView(roll());
		expect(v.modifiers).toEqual([
			{ source: 'Strength', value: '+3' },
			{ source: 'Penalty', value: '-1' }
		]);
	});

	it('shows the server total unchanged', () => {
		// 12 + 3 - 1 would be 14; the card must show the server's number.
		expect(rollCardView(roll({ total: 99 })).total).toBe(99);
	});

	it('uses the label as the title and keeps the notation underneath', () => {
		const v = rollCardView(roll({ label: 'Longsword attack' }));
		expect(v.title).toBe('Longsword attack');
		expect(v.notation).toBe('2d20kh1+5');
	});

	it('uses the notation as the title when there is no label', () => {
		const v = rollCardView(roll({ label: '  ' }));
		expect(v.title).toBe('2d20kh1+5');
		expect(v.notation).toBe('');
	});

	it('carries critical, fumble and the roll mode', () => {
		const v = rollCardView(roll({ critical: true, fumble: false }));
		expect(v.critical).toBe(true);
		expect(v.fumble).toBe(false);
		expect(v.modeText).toBe('Advantage');
		expect(rollCardView(roll({ fumble: true })).fumble).toBe(true);
	});

	it('flags a GM-only roll', () => {
		expect(rollCardView(roll({ visibility: RollVisibility.GM_ONLY })).hidden).toBe(true);
		expect(rollCardView(roll({ visibility: RollVisibility.PUBLIC })).hidden).toBe(false);
		expect(rollCardView(roll()).hidden).toBe(false);
	});

	it('treats text from the server as plain text', () => {
		const v = rollCardView(roll({ label: '<b>x</b>' }));
		expect(v.title).toBe('<b>x</b>');
	});
});

describe('modeText', () => {
	it('is empty for a normal roll', () => {
		expect(modeText(RollMode.NORMAL)).toBe('');
		expect(modeText(RollMode.UNSPECIFIED)).toBe('');
		expect(modeText(RollMode.DISADVANTAGE)).toBe('Disadvantage');
	});
});

describe('rollAnnouncement', () => {
	it('reads as one sentence', () => {
		const v = rollCardView(roll({ label: 'Stealth', critical: true }));
		expect(rollAnnouncement('Pip', v)).toBe('Pip rolled Stealth: 17, critical');
		expect(rollAnnouncement('Pip', rollCardView(roll({ label: '' })))).toBe(
			'Pip rolled 2d20kh1+5: 17'
		);
	});
});
