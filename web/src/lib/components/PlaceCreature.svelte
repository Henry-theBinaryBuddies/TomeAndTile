<script lang="ts">
	import { getGameClient } from '../net';
	import { rejectionText } from '../chat';
	import {
		buildCreateTokenKind,
		creatureEntries,
		placeBlockedReason
	} from '../gmtools';

	// Puts a creature on the active map. The token appears when the server's TokenUpserted
	// arrives; this list never adds it by itself.
	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	let filter = $state('');
	let column = $state('0');
	let row = $state('0');
	let sentIds = $state<string[]>([]);
	let note = $state('');

	const entries = $derived(creatureEntries($gameState.creatures, filter));
	const blocked = $derived(placeBlockedReason($gameState.mapTarget, column, row));
	const rejection = $derived(
		$lastRejection && sentIds.includes($lastRejection.clientIntentId)
			? rejectionText($lastRejection.code)
			: ''
	);

	function place(creatureId: string) {
		const kind = buildCreateTokenKind($gameState.mapTarget, creatureId, column, row);
		if (!kind) return;
		const id = client.send(kind);
		if (id) {
			sentIds = [...sentIds, id];
			note = 'Sent to the server.';
		} else note = 'Not connected. Nothing was sent.';
	}
</script>

<section class="place" aria-labelledby="place-title">
	<h3 id="place-title">Put a creature on the map</h3>
	<div class="where">
		<label>Column <input class="num" inputmode="numeric" autocomplete="off" bind:value={column} /></label>
		<label>Row <input class="num" inputmode="numeric" autocomplete="off" bind:value={row} /></label>
	</div>
	<p class="note" id="place-reason">{blocked}</p>
	<label class="filter">
		Filter by name
		<input type="search" autocomplete="off" bind:value={filter} />
	</label>
	{#if entries.length === 0}
		<p class="note">{filter.trim() ? 'No creature matches.' : 'There are no creatures yet. Import one.'}</p>
	{:else}
		<ul class="creatures" aria-label="Creatures">
			{#each entries as entry (entry.creatureId)}
				<li>
					<span class="name">{entry.name}</span>
					{#if entry.summary}<span class="summary">{entry.summary}</span>{/if}
					<button
						type="button"
						disabled={blocked !== ''}
						aria-describedby="place-reason"
						aria-label="Place {entry.name}"
						onclick={() => place(entry.creatureId)}>Place</button
					>
				</li>
			{/each}
		</ul>
	{/if}
	<p class="note" role="status" aria-live="polite">{rejection ? '' : note}</p>
	{#if rejection}<p class="error" role="alert">{rejection}</p>{/if}
</section>

<style>
	.place {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		min-width: 0;
	}
	h3 {
		margin: 0;
		font-size: 1rem;
	}
	.where {
		display: flex;
		gap: 0.8rem;
		flex-wrap: wrap;
	}
	label {
		display: flex;
		gap: 0.4rem;
		align-items: center;
	}
	.filter {
		flex-wrap: wrap;
	}
	.num {
		width: 4rem;
	}
	.creatures {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	li {
		display: flex;
		flex-wrap: wrap;
		gap: 0.2rem 0.6rem;
		align-items: baseline;
	}
	.name {
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.summary {
		color: var(--muted);
		font-size: 0.85rem;
		overflow-wrap: anywhere;
	}
	.note {
		margin: 0;
		color: var(--muted);
		min-height: 1.2em;
	}
	.error {
		margin: 0;
		color: #ff9e9e;
	}
</style>
