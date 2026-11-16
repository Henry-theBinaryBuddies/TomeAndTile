<script lang="ts">
	import { getGameClient } from '../net';
	import { currentEncounter, initiativeView, turnAnnouncement } from '../initiative';

	const { state: panelState } = getGameClient();

	const encounter = $derived(currentEncounter($panelState.encounters, $panelState.activeSceneId));
	const view = $derived(encounter ? initiativeView(encounter, $panelState.tokenNames) : null);
</script>

{#if view}
	<div class="initiative">
		<p class="round">Round <strong>{view.round}</strong></p>
		<!-- Spoken when the turn or the round changes. -->
		<p class="sr-only" aria-live="polite" aria-atomic="true">{turnAnnouncement(view)}</p>
		<ol class="order" aria-label="Turn order">
			{#each view.rows as row (row.index)}
				<li class:current={row.current} aria-current={row.current ? 'true' : undefined}>
					{#if row.current}<span class="marker" aria-hidden="true">▶</span>{/if}
					<span class="name">{row.name}</span>
					<span class="score" title="Initiative">{row.initiative}</span>
					{#if row.current}<span class="sr-only">(current turn)</span>{/if}
				</li>
			{/each}
		</ol>
	</div>
{:else}
	<p class="empty">No combat in progress.</p>
{/if}

<style>
	.initiative {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 0.9rem;
		align-items: center;
	}
	.round {
		margin: 0;
		color: var(--muted);
	}
	.round strong {
		color: var(--text);
	}
	.order {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 0;
		padding: 0;
		list-style: none;
		min-width: 0;
	}
	li {
		display: flex;
		gap: 0.45rem;
		align-items: baseline;
		max-width: 100%;
		padding: 0.3rem 0.65rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 999px;
	}
	li.current {
		border-color: var(--accent);
		box-shadow: inset 0 0 0 1px var(--accent);
		font-weight: 600;
	}
	.marker {
		color: var(--accent);
	}
	.name {
		overflow-wrap: anywhere;
	}
	.score {
		color: var(--muted);
		font-size: 0.85rem;
		font-weight: 400;
	}
	.empty {
		margin: 0;
		color: var(--muted);
	}
</style>
