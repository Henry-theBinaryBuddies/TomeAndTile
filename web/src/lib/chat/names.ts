import type { Member } from '@tomeandtile/contracts';

/** The display name for a user id, or "Unknown" if the server did not say. */
export function displayName(members: readonly Member[], userId: string): string {
	const name = members.find((m) => m.userId === userId)?.displayName.trim();
	return name || 'Unknown';
}
