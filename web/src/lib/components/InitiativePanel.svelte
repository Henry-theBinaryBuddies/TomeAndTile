<script lang="ts">
	import { tick } from 'svelte';
	import { getGameClient } from '../net';
	import { canEditConditions, conditionChanges, conditionList, shortList } from '../conditions';
	import ConditionEditor from './ConditionEditor.svelte';
	import HealthBadge from './HealthBadge.svelte';
	import { currentEncounter, initiativeView, turnAnnouncement } from '../initiative';

	const { state: panelState } = getGameClient();

	const encounter = $derived(currentEncounter($panelState.encounters, $panelState.activeSceneId));
	const view = $derived(encounter ? initiativeView(encounter, $panelState.tokens) : null);
	const isGm = $derived(canEditConditions($panelState.role));

	// Which row's full condition list is open, and which row's editor.
	let expanded = $state<number | null>(null);
	let editing = $state<{ tokenId: string; index: number } | null>(null);
	async function closeEditor() {
		const index = editing?.index;
		editing = null;
		await tick();
		document.querySelector<HTMLButtonElement>(`[data-edit-conditions="${index}"]`)?.focus();
	}

	// Said once when a token's conditions change. The first state is never announced.
	let announcement = $state('');
	let seen: typeof $panelState.tokens | null = null;
	$effect(() => {
		const now = $panelState.tokens;
		if (seen && seen !== now) {
			const said = conditionChanges(seen, now);
			if (said.length > 0) announcement = said.join('. ');
		}
		seen = now;
	});
</script>

{#if view}
	<div class="initiative">
		<p class="round">Round <strong>{view.round}</strong></p>
		<!-- Spoken when the turn or the round changes. -->
		<p class="sr-only" aria-live="polite" aria-atomic="true">{turnAnnouncement(view)}</p>
		<ol class="order" aria-label="Turn order">
			{#each view.rows as row (row.index)}
				{@const list = conditionList($panelState.tokens[row.tokenId])}
				{@const short = shortList(list.items)}
				<li class:current={row.current} aria-current={row.current ? 'true' : undefined}>
					{#if row.current}<span class="marker" aria-hidden="true">▶</span>{/if}
					<span class="name">{row.name}</span>
					<span class="score" title="Initiative">{row.initiative}</span>
					<HealthBadge view={row.health} />
					{#if row.current}<span class="sr-only">(current turn)</span>{/if}
					{#if list.items.length > 0}
						<!-- Everything for screen readers; the visible short list is hidden from them. -->
						<span class="sr-only">Conditions: {short.full}</span>
						<span class="conds" aria-hidden="true" title={short.full}>
							{(expanded === row.index ? list.items : short.shown).map((i) => i.name).join(', ')}
						</span>
						{#if short.hidden > 0}
							<button
								type="button"
								class="more"
								aria-expanded={expanded === row.index}
								aria-label={`Show all ${list.items.length} conditions for ${row.name}`}
								onclick={() => (expanded = expanded === row.index ? null : row.index)}
							>
								{expanded === row.index ? 'less' : `+${short.hidden}`}
							</button>
						{/if}
					{/if}
					{#if isGm}
						<button
							type="button"
							class="more"
							data-edit-conditions={row.index}
							aria-label={`Edit conditions for ${row.name}`}
							onclick={() => (editing = { tokenId: row.tokenId, index: row.index })}
						>
							Edit
						</button>
					{/if}
				</li>
			{/each}
		</ol>
		<p class="sr-only" aria-live="polite" aria-atomic="true">{announcement}</p>
		{#if editing}
			<ConditionEditor tokenId={editing.tokenId} onclose={closeEditor} />
		{/if}
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
	.conds {
		color: var(--muted);
		font-size: 0.85rem;
		font-weight: 400;
		overflow-wrap: anywhere;
	}
	.more {
		min-height: 1.6rem;
		padding: 0 0.4rem;
		font-size: 0.8rem;
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
