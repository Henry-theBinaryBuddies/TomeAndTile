// Test helper: loads the recorded sample data in contracts/fixtures and parses it with the
// generated types, so no test needs a running server. Uses Vite's `?raw` imports, so it needs
// no Node types.
import { fromJson, type JsonValue } from '@bufbuild/protobuf';
import { CreatureSchema, SnapshotSchema, type Creature, type Snapshot } from '@tomeandtile/contracts';
import pregenTable from '../../../../contracts/fixtures/seeds/pregen-table.json?raw';

const characters = import.meta.glob('../../../../contracts/fixtures/characters/*.json', {
	query: '?raw',
	import: 'default',
	eager: true
}) as Record<string, string>;

export const CHARACTER_FILES = Object.keys(characters)
	.map((path) => path.replace(/^.*\/([^/]+)\.json$/, '$1'))
	.sort();

export function loadCharacter(name: string): Creature {
	const path = Object.keys(characters).find((p) => p.endsWith(`/${name}.json`));
	if (!path) throw new Error(`no character fixture named ${name}`);
	return fromJson(CreatureSchema, JSON.parse(characters[path]) as JsonValue);
}

export function loadPregenTable(): Snapshot {
	return fromJson(SnapshotSchema, JSON.parse(pregenTable) as JsonValue);
}
