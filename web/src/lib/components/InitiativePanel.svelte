<script lang="ts">
	import { tick } from 'svelte';
	import { getGameClient, type IntentKind } from '../net';
	import { rejectionText } from '../chat';
	import { canEditConditions, conditionChanges, conditionList, shortList } from '../conditions';
	import ConditionEditor from './ConditionEditor.svelte';
	import HealthBadge from './HealthBadge.svelte';
	import StartEncounter from './StartEncounter.svelte';
	import {
		buildEndEncounterKind,
		buildEndTurnKind,
		buildSetInitiativeKind,
		canControlEncounter,
		canEndTurn,
		canMove,
		currentEncounter,
		initiativeView,
		moveChanges,
		parseInitiative,
		turnAnnouncement,
		type MoveDirection
	} from '../initiative';

	const client = getGameClient();
	const { state: panelState, lastRejection } = client;

	const encounter = $derived(currentEncounter($panelState.encounters, $panelState.activeSceneId));
	const view = $derived(encounter ? initiativeView(encounter, $panelState.tokens) : null);
	const isGm = $derived(canEditConditions($panelState.role));
	const gmEncounter = $derived(canControlEncounter($panelState.role));
	const currentToken = $derived(
		encounter ? $panelState.tokens[encounter.combatants[encounter.turnIndex]?.tokenId ?? ''] : undefined
	);
	const showEndTurn = $derived(
		encounter !== null &&
			encounter.combatants[encounter.turnIndex] !== undefined &&
			canEndTurn($panelState.role, $panelState.userId, currentToken)
	);

	// What was last sent from this bar, so a rejection of it can be shown here.
	let sentIds = $state<string[]>([]);
	let note = $state('');
	const rejection = $derived(
		$lastRejection && sentIds.includes($lastRejection.clientIntentId)
			? rejectionText($lastRejection.code)
			: ''
	);
	function send(kind: IntentKind) {
		note = '';
		const id = client.send(kind);
		if (id) sentIds = [...sentIds, id];
		else note = 'Not connected. Nothing was sent.';
	}

	let confirmingEnd = $state(false);
	let editingOrder = $state(false);
	// Draft initiative text per row, shown only while the order is being edited.
	let drafts = $state<Record<number, string>>({});
	function draftFor(index: number, initiative: number): string {
		return drafts[index] ?? String(initiative);
	}
	function setInitiative(index: number, tokenId: string, order: number) {
		if (!encounter) return;
		const parsed = parseInitiative(drafts[index] ?? '');
		if (!parsed.ok) {
			note = parsed.reason;
			return;
		}
		send(buildSetInitiativeKind(encounter.encounterId, tokenId, parsed.value, order));
		delete drafts[index];
	}
	function move(index: number, dir: MoveDirection) {
		if (!encounter) return;
		for (const c of moveChanges(encounter.combatants, index, dir))
			send(buildSetInitiativeKind(encounter.encounterId, c.tokenId, c.initiative, c.order));
	}
	function onMoveKey(e: KeyboardEvent, index: number) {
		if (e.isComposing || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
		e.preventDefault();
		move(index, e.key === 'ArrowUp' ? 'up' : 'down');
	}

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
					{#if editingOrder && gmEncounter}
						<form
							class="edit"
							onsubmit={(e) => {
								e.preventDefault();
								setInitiative(row.index, row.tokenId, encounter?.combatants[row.index]?.order ?? 0);
							}}
						>
							<label class="sr-only" for="init-{row.index}">Initiative for {row.name}</label>
							<input
								id="init-{row.index}"
								class="num"
								inputmode="numeric"
								autocomplete="off"
								value={draftFor(row.index, row.initiative)}
								oninput={(e) => (drafts[row.index] = e.currentTarget.value)}
								onkeydown={(e) => {
									if (e.key === 'Escape' && !e.isComposing) {
										e.preventDefault();
										e.stopPropagation();
										delete drafts[row.index];
										e.currentTarget.value = String(row.initiative);
									}
								}}
							/>
							<button type="submit" class="more">Set</button>
							{#if encounter}
								<button
									type="button"
									class="more"
									disabled={!canMove(encounter.combatants, row.index, 'up')}
									aria-label={`Move ${row.name} up among equal initiatives`}
									onkeydown={(e) => onMoveKey(e, row.index)}
									onclick={() => move(row.index, 'up')}>▲</button
								>
								<button
									type="button"
									class="more"
									disabled={!canMove(encounter.combatants, row.index, 'down')}
									aria-label={`Move ${row.name} down among equal initiatives`}
									onkeydown={(e) => onMoveKey(e, row.index)}
									onclick={() => move(row.index, 'down')}>▼</button
								>
							{/if}
						</form>
					{:else}
						<span class="score" title="Initiative">{row.initiative}</span>
					{/if}
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
							Conditions
						</button>
					{/if}
				</li>
			{/each}
		</ol>
		<p class="sr-only" aria-live="polite" aria-atomic="true">{announcement}</p>
		{#if encounter && (showEndTurn || gmEncounter)}
			<div class="controls">
				{#if showEndTurn}
					<button type="button" onclick={() => send(buildEndTurnKind(encounter.encounterId))}>
						End turn
					</button>
				{/if}
				{#if gmEncounter}
					<button type="button" aria-pressed={editingOrder} onclick={() => (editingOrder = !editingOrder)}>
						{editingOrder ? 'Done editing order' : 'Edit order'}
					</button>
					{#if confirmingEnd}
						<span role="group" aria-label="Confirm ending the encounter">
							End the encounter?
							<button
								type="button"
								onclick={() => {
									send(buildEndEncounterKind(encounter.encounterId));
									confirmingEnd = false;
								}}>Yes, end it</button
							>
							<button type="button" onclick={() => (confirmingEnd = false)}>Keep going</button>
						</span>
					{:else}
						<button type="button" onclick={() => (confirmingEnd = true)}>End encounter</button>
					{/if}
				{/if}
			</div>
		{/if}
		<p class="note" role="status" aria-live="polite">{note}</p>
		{#if rejection}<p class="error" role="alert">{rejection}</p>{/if}
		{#if editing}
			<ConditionEditor tokenId={editing.tokenId} onclose={closeEditor} />
		{/if}
	</div>
{:else if gmEncounter}
	<StartEncounter />
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
		/* A row with health, conditions and GM controls wraps onto more lines instead of
		   squeezing the name into a column of single letters on a phone (#113). */
		flex-wrap: wrap;
		gap: 0.6rem;
		align-items: center;
		max-width: 100%;
		min-width: 0;
		padding: 0.3rem 0.8rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 1.2rem;
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
		flex: 0 1 auto;
		min-width: min(100%, 5rem);
		overflow-wrap: break-word;
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
	.controls {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		align-items: center;
		flex-basis: 100%;
	}
	.edit {
		display: flex;
		gap: 0.25rem;
		align-items: center;
	}
	.num {
		width: 3.5rem;
		min-height: 1.8rem;
	}
	.note {
		margin: 0;
		color: var(--muted);
	}
	.error {
		margin: 0;
		color: #ff9e9e;
	}
	/* On a phone each combatant is a full-width row, so the list reads as a list. */
	@media (max-width: 640px) {
		.order {
			flex-direction: column;
			flex-wrap: nowrap;
		}
		li {
			width: 100%;
		}
	}
	.empty {
		margin: 0;
		color: var(--muted);
	}
</style>
