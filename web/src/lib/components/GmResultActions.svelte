<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { Role } from '@tomeandtile/contracts';
	import { getGameClient } from '../net';
	import {
		buildUndoKind,
		buildUseAgainKind,
		rejectionText,
		rememberedUse,
		rerollBlockedReason,
		rerollNext,
		showOverrides,
		undoQuestion,
		type ResultCardView
	} from '../chat';

	// The GM's Undo and Re-roll for one result card. The card says "Undone" only when the server's
	// EventUndone arrives. Re-roll is Undo, then the same use again once the server has undone it:
	// if the undo is refused, nothing else is sent.
	let {
		view,
		seqs,
		clientIntentId,
		undone
	}: { view: ResultCardView; seqs: readonly bigint[]; clientIntentId: string; undone: boolean } = $props();

	const client = getGameClient();
	const { state: gameState, lastRejection } = client;

	const show = $derived(showOverrides($gameState.role, undone));
	const targetNames = $derived(view.targets.map((t) => t.name));
	const rerollWhy = $derived(rerollBlockedReason(clientIntentId));

	let asking = $state<'undo' | 'reroll' | null>(null);
	let sentIds = $state<string[]>([]);
	let undoId = $state<string | null>(null);
	let rerolling = $state(false);
	let note = $state('');
	let undoButton = $state<HTMLButtonElement>();
	let rerollButton = $state<HTMLButtonElement>();

	const rejection = $derived(
		$lastRejection && sentIds.includes($lastRejection.clientIntentId)
			? rejectionText($lastRejection.code)
			: ''
	);
	const undoRefused = $derived(
		undoId !== null && $lastRejection !== null && $lastRejection.clientIntentId === undoId
	);

	async function stopAsking() {
		const was = asking;
		asking = null;
		await tick();
		(was === 'reroll' ? rerollButton : undoButton)?.focus();
	}

	function sendUndo(): string | null {
		const kind = buildUndoKind(seqs);
		if (!kind) {
			note = 'This result has no event to undo.';
			return null;
		}
		const id = client.send(kind);
		if (!id) {
			note = 'Not connected. Nothing was sent.';
			return null;
		}
		sentIds = [...sentIds, id];
		undoId = id;
		note = 'Waiting for the server…';
		return id;
	}

	function confirm() {
		const was = asking;
		asking = null;
		if (!sendUndo()) return;
		rerolling = was === 'reroll';
	}

	// Re-roll: once the server has undone the card, send the use again, once.
	$effect(() => {
		if (!rerolling) return;
		const next = rerollNext(undoRefused, undone);
		if (next === 'wait') return;
		rerolling = false;
		if (next === 'stop') {
			note = '';
			return;
		}
		const use = rememberedUse(clientIntentId);
		if (!use) return;
		untrack(() => {
			const id = client.send(buildUseAgainKind(use));
			if (id) {
				sentIds = [...sentIds, id];
				note = 'Rolled again. The new result is below.';
			} else note = 'Not connected. The use was not sent again.';
		});
	});
</script>

{#if $gameState.role === Role.GM && (show || rerolling || rejection || note)}
	<div class="gm">
		{#if show}
			{#if asking}
				<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
				<div
					role="group"
					aria-label={undoQuestion(view.abilityName, view.sourceName, targetNames)}
					onkeydown={(e) => {
						if (e.key === 'Escape' && !e.isComposing) {
							e.preventDefault();
							e.stopPropagation();
							stopAsking();
						}
					}}
				>
					<span>
						{asking === 'reroll'
							? `${undoQuestion(view.abilityName, view.sourceName, targetNames).slice(0, -1)} and roll it again?`
							: undoQuestion(view.abilityName, view.sourceName, targetNames)}
					</span>
					<button type="button" onclick={confirm}>Yes</button>
					<button type="button" onclick={stopAsking}>Keep it</button>
				</div>
			{:else}
				<button
					type="button"
					bind:this={undoButton}
					onclick={async () => {
						asking = 'undo';
						await tick();
						document.querySelector<HTMLButtonElement>('.gm [role="group"] button:last-of-type')?.focus();
					}}>Undo</button
				>
				<button
					type="button"
					bind:this={rerollButton}
					disabled={rerollWhy !== ''}
					aria-describedby={rerollWhy ? `reroll-why-${clientIntentId}` : undefined}
					onclick={async () => {
						asking = 'reroll';
						await tick();
						document.querySelector<HTMLButtonElement>('.gm [role="group"] button:last-of-type')?.focus();
					}}>Re-roll</button
				>
				{#if rerollWhy}<span class="why" id="reroll-why-{clientIntentId}">{rerollWhy}</span>{/if}
			{/if}
		{/if}
		<p class="note" role="status" aria-live="polite">{rejection ? '' : note}</p>
		{#if rejection}<p class="error" role="alert">{rejection}</p>{/if}
	</div>
{/if}

<style>
	.gm {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 0.5rem;
		align-items: center;
		margin-top: 0.4rem;
	}
	.gm button {
		min-height: 1.8rem;
		font-size: 0.85rem;
	}
	.why {
		color: var(--muted);
		font-size: 0.8rem;
	}
	.note {
		flex-basis: 100%;
		margin: 0;
		color: var(--muted);
		font-size: 0.9rem;
	}
	.error {
		flex-basis: 100%;
		margin: 0;
		color: #ff9e9e;
	}
</style>
