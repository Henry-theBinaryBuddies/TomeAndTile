import { describe, expect, it } from 'vitest';
import { PANEL_IDS } from './model';
import { isSheetOpen, nextOpen } from './viewport';

describe('phone sheets', () => {
	it('opens a panel from a closed state and closes the open one on a second press', () => {
		expect(nextOpen(null, 'chat')).toBe('chat');
		expect(nextOpen('chat', 'chat')).toBeNull();
	});

	it('switches straight to another panel', () => {
		expect(nextOpen('chat', 'sheet')).toBe('sheet');
	});

	it('marks exactly one panel open, and none when closed', () => {
		for (const open of PANEL_IDS) {
			expect(PANEL_IDS.filter((id) => isSheetOpen(open, id))).toEqual([open]);
		}
		expect(PANEL_IDS.filter((id) => isSheetOpen(null, id))).toEqual([]);
	});
});
