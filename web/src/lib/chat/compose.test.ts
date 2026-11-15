import { create } from '@bufbuild/protobuf';
import { describe, expect, it } from 'vitest';
import { IntentRejectedSchema, RollMode, RollVisibility } from '@tomeandtile/contracts';
import { buildChatKind, buildRollKind, textAfterRejection } from './compose';

describe('buildChatKind', () => {
	it('does not send empty or blank messages', () => {
		expect(buildChatKind('')).toBeNull();
		expect(buildChatKind('   \n ')).toBeNull();
	});

	it('sends the trimmed text', () => {
		const kind = buildChatKind('  hello there ');
		expect(kind?.case).toBe('chat');
		if (kind?.case === 'chat') expect(kind.value.text).toBe('hello there');
	});

	it('sends text as typed, without turning it into HTML', () => {
		const kind = buildChatKind('<b>hi</b>');
		if (kind?.case === 'chat') expect(kind.value.text).toBe('<b>hi</b>');
	});
});

describe('buildRollKind', () => {
	const form = { notation: ' d20+5 ', mode: RollMode.ADVANTAGE, label: ' Stealth ', hidden: false };

	it('does not send an empty notation', () => {
		expect(buildRollKind({ ...form, notation: '  ' }, false)).toBeNull();
	});

	it('carries notation, mode and label', () => {
		const kind = buildRollKind(form, false);
		expect(kind?.case).toBe('rollDice');
		if (kind?.case !== 'rollDice') return;
		expect(kind.value.notation).toBe('d20+5');
		expect(kind.value.mode).toBe(RollMode.ADVANTAGE);
		expect(kind.value.label).toBe('Stealth');
	});

	it('is public for a player even if the hidden box was ticked', () => {
		const kind = buildRollKind({ ...form, hidden: true }, false);
		if (kind?.case === 'rollDice') expect(kind.value.visibility).toBe(RollVisibility.PUBLIC);
	});

	it('is GM-only only for the GM who asked for it', () => {
		const hidden = buildRollKind({ ...form, hidden: true }, true);
		const open = buildRollKind({ ...form, hidden: false }, true);
		if (hidden?.case === 'rollDice') expect(hidden.value.visibility).toBe(RollVisibility.GM_ONLY);
		if (open?.case === 'rollDice') expect(open.value.visibility).toBe(RollVisibility.PUBLIC);
	});
});

describe('textAfterRejection', () => {
	const sent = { id: 'a', text: 'too long...' };
	const rejection = (id: string) =>
		create(IntentRejectedSchema, { clientIntentId: id, code: 'chat_too_long' });

	it('puts the text back when its message was refused and the box is empty', () => {
		expect(textAfterRejection(sent, rejection('a'), '')).toBe('too long...');
	});

	it('does not overwrite what has been typed since', () => {
		expect(textAfterRejection(sent, rejection('a'), 'new text')).toBe('new text');
	});

	it('ignores a rejection of some other intent', () => {
		expect(textAfterRejection(sent, rejection('b'), '')).toBe('');
	});

	it('changes nothing without a rejection or a sent message', () => {
		expect(textAfterRejection(sent, null, 'x')).toBe('x');
		expect(textAfterRejection(null, rejection('a'), 'x')).toBe('x');
	});
});
