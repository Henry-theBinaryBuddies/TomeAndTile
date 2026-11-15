import { describe, expect, it } from 'vitest';
import { rejectionText } from './rejections';
import { displayName } from './names';
import { create } from '@bufbuild/protobuf';
import { MemberSchema } from '@tomeandtile/contracts';

describe('rejectionText', () => {
	it('has plain-language text for every chat and dice code the servers send', () => {
		const codes = [
			'empty_chat',
			'chat_too_long',
			'not_permitted',
			'bad_notation',
			'dice_empty',
			'dice_syntax',
			'dice_bad_sides',
			'dice_bad_count',
			'dice_too_many_dice',
			'dice_too_many_terms',
			'dice_bad_keep',
			'dice_number_too_large',
			'dice_too_long'
		];
		for (const code of codes) {
			const text = rejectionText(code);
			expect(text, code).not.toMatch(/could not do that/);
			expect(text, code).not.toContain('_');
		}
	});

	it('names an unknown code instead of hiding it', () => {
		expect(rejectionText('something_new')).toBe('The server could not do that (something_new).');
		expect(rejectionText('')).toContain('no reason given');
	});
});

describe('displayName', () => {
	const members = [
		create(MemberSchema, { userId: 'u1', displayName: 'Gale' }),
		create(MemberSchema, { userId: 'u2', displayName: '  ' })
	];
	it('finds the name by user id', () => {
		expect(displayName(members, 'u1')).toBe('Gale');
	});
	it('says Unknown, never a guess, when the name is missing', () => {
		expect(displayName(members, 'u2')).toBe('Unknown');
		expect(displayName(members, 'nobody')).toBe('Unknown');
		expect(displayName(members, '')).toBe('Unknown');
	});
});
