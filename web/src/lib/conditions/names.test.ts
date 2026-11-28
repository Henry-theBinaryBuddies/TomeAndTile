import { describe, expect, it } from 'vitest';
import { SRD_CONDITIONS, conditionName } from './names';

describe('conditionName', () => {
	it('has a name and a one-line meaning for every SRD condition', () => {
		for (const id of SRD_CONDITIONS) {
			const c = conditionName(id);
			expect(c.srd).toBe(true);
			expect(c.name.toLowerCase()).toBe(id);
			expect(c.meaning).not.toBe('');
			expect(c.meaning).not.toContain('\n');
		}
		expect(conditionName('prone').name).toBe('Prone');
	});

	it('shows an unknown id as sent, with no meaning, and never drops it', () => {
		const c = conditionName('hunters_mark');
		expect(c).toEqual({ id: 'hunters_mark', name: 'hunters_mark', meaning: '', srd: false });
	});

	it('does not take ids from the object prototype', () => {
		expect(conditionName('constructor').srd).toBe(false);
		expect(conditionName('toString').name).toBe('toString');
	});

	it('keeps markup in an id as plain text', () => {
		expect(conditionName('<b>x</b>').name).toBe('<b>x</b>');
	});
});
