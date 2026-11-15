import { describe, expect, it } from 'vitest';
import { Role } from '@tomeandtile/contracts';
import { applyEvent, emptyState, foldMessage, MAX_LOG_ENTRIES } from './state';
import {
	chatKind,
	encounterEndedKind,
	encounterUpdatedKind,
	eventMessage,
	rollKind,
	sceneUpsertedKind,
	snapshotMessage,
	tokenRemovedKind,
	tokenUpsertedKind,
	undoneKind
} from './testing';

const joined = () => foldMessage(emptyState(), snapshotMessage(5n));
const fold = (state: ReturnType<typeof joined>, seq: bigint, kind: Parameters<typeof eventMessage>[1]) =>
	foldMessage(state, eventMessage(seq, kind));

describe('snapshot', () => {
	it('sets identity, role, members, scene and encounters', () => {
		const s = joined();
		expect(s.joined).toBe(true);
		expect(s.seq).toBe(5n);
		expect(s.userId).toBe('mock-player');
		expect(s.role).toBe(Role.PLAYER);
		expect(s.members.map((m) => m.displayName)).toEqual(['Gale', 'Pip']);
		expect(s.activeSceneId).toBe('scene-1');
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-1']);
		expect(s.log).toEqual([]);
	});

	it('replaces earlier state, including the log', () => {
		let s = fold(joined(), 6n, chatKind('mock-gm', 'hello'));
		expect(s.log).toHaveLength(1);
		s = foldMessage(s, snapshotMessage(20n));
		expect(s.seq).toBe(20n);
		expect(s.log).toEqual([]);
	});

	it('starts not joined', () => {
		expect(emptyState().joined).toBe(false);
	});
});

describe('events', () => {
	it('are ignored before the first snapshot', () => {
		const s = fold(emptyState(), 1n, chatKind('u', 'early'));
		expect(s).toEqual(emptyState());
	});

	it('append chat lines and rolls in order', () => {
		let s = fold(joined(), 6n, chatKind('mock-gm', 'hello'));
		s = fold(s, 7n, rollKind('r1', 17));
		expect(s.seq).toBe(7n);
		expect(s.log.map((e) => e.kind)).toEqual(['chat', 'roll']);
		const [chat, roll] = s.log;
		expect(chat.kind === 'chat' && chat.text).toBe('hello');
		expect(chat.kind === 'chat' && chat.userId).toBe('mock-gm');
		expect(roll.kind === 'roll' && roll.roll.total).toBe(17);
	});

	it('at or below the last applied sequence change nothing', () => {
		const s = fold(joined(), 6n, chatKind('u', 'once'));
		expect(fold(s, 6n, chatKind('u', 'again'))).toBe(s);
		expect(fold(s, 3n, chatKind('u', 'old'))).toBe(s);
	});

	it('does not change the input state', () => {
		const before = joined();
		const snapshot = structuredClone({ ...before, log: [...before.log] });
		fold(before, 6n, chatKind('u', 'x'));
		expect(before.log).toEqual(snapshot.log);
		expect(before.seq).toBe(5n);
	});

	it('track encounters: add, update and end', () => {
		let s = fold(joined(), 6n, encounterUpdatedKind('enc-1', 2));
		expect(s.encounters).toHaveLength(1);
		expect(s.encounters[0].round).toBe(2);
		s = fold(s, 7n, encounterUpdatedKind('enc-2', 1));
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-1', 'enc-2']);
		s = fold(s, 8n, encounterEndedKind('enc-1'));
		expect(s.encounters.map((e) => e.encounterId)).toEqual(['enc-2']);
	});

	it('mark an undone entry', () => {
		let s = fold(joined(), 6n, chatKind('u', 'oops'));
		s = fold(s, 7n, undoneKind(6n));
		expect(s.log[0].undone).toBe(true);
	});

	it('that are not panel state only advance the sequence', () => {
		const s = fold(joined(), 6n, sceneUpsertedKind());
		expect(s.seq).toBe(6n);
		expect(s.log).toEqual([]);
	});

	it('keep token names: set by the Snapshot, changed by upserts, dropped by removals', () => {
		let s = joined();
		expect(s.tokenNames).toEqual({ 't-aria': 'Aria', 't-gob': 'Goblin' });
		s = fold(s, 6n, tokenUpsertedKind('t-gob', 'Goblin Boss'));
		s = fold(s, 7n, tokenUpsertedKind('t-new', 'Orc'));
		expect(s.tokenNames).toEqual({ 't-aria': 'Aria', 't-gob': 'Goblin Boss', 't-new': 'Orc' });
		s = fold(s, 8n, tokenRemovedKind('t-aria'));
		expect(s.tokenNames).toEqual({ 't-gob': 'Goblin Boss', 't-new': 'Orc' });
	});

	it('keep at most MAX_LOG_ENTRIES, dropping the oldest', () => {
		let s = joined();
		for (let i = 1; i <= MAX_LOG_ENTRIES + 5; i++) {
			s = applyEvent(s, eventMessage(5n + BigInt(i), chatKind('u', `m${i}`)).kind.value as never);
		}
		expect(s.log).toHaveLength(MAX_LOG_ENTRIES);
		const first = s.log[0];
		expect(first.kind === 'chat' && first.text).toBe('m6');
	});
});
