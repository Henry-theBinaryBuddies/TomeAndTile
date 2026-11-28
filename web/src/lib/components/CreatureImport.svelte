<script lang="ts">
	import { getGameClient, getUploadSession } from '../net';
	import { httpBaseFromSocketUrl } from '../upload';
	import { importCreature, importFailureText, type ImportOutcome } from '../gmtools';

	// The file goes to the server, which runs the importer and stores the creature. The list of
	// creatures changes when the server's CreatureUpserted arrives, not from this answer.
	const { state: gameState } = getGameClient();

	let picker = $state<HTMLInputElement>();
	let pending = $state(false);
	let outcome = $state<ImportOutcome | null>(null);

	async function onPick(e: Event) {
		const input = e.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		const campaignId = $gameState.mapTarget?.campaignId ?? '';
		if (!file || pending) return;
		if (campaignId === '') {
			outcome = { ok: false, reason: 'unknown_campaign' };
			return;
		}
		pending = true;
		outcome = null;
		const session = getUploadSession();
		outcome = await importCreature({
			baseUrl: httpBaseFromSocketUrl(session.socketUrl),
			campaignId,
			token: session.token,
			file
		});
		pending = false;
	}
</script>

<section class="import" aria-labelledby="import-title">
	<h3 id="import-title">Import a creature</h3>
	<input
		bind:this={picker}
		type="file"
		accept=".json,application/json"
		class="sr-only"
		id="import-file"
		onchange={onPick}
		disabled={pending}
	/>
	<button type="button" disabled={pending} onclick={() => picker?.click()}>Choose a creature file</button>
	<p class="note" role="status" aria-live="polite">
		{#if pending}Sending the file to the server…{/if}
		{#if outcome?.ok}
			{outcome.replaced ? 'Replaced' : 'Imported'} {outcome.name}.
			{outcome.notes.length === 0 ? 'The server had nothing to report.' : 'The server noted:'}
		{/if}
	</p>
	{#if outcome?.ok && outcome.notes.length > 0}
		<ul class="notes" aria-label="Import report">
			{#each outcome.notes as text, i (i)}<li>{text}</li>{/each}
		</ul>
	{/if}
	{#if outcome && !outcome.ok}
		<p class="error" role="alert">{importFailureText(outcome.reason)}</p>
	{/if}
</section>

<style>
	.import {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		align-items: flex-start;
	}
	h3 {
		margin: 0;
		font-size: 1rem;
	}
	.note {
		margin: 0;
		color: var(--muted);
		min-height: 1.2em;
	}
	.notes {
		margin: 0;
		padding-left: 1.2rem;
		overflow-wrap: anywhere;
	}
	.error {
		margin: 0;
		color: #ff9e9e;
		overflow-wrap: anywhere;
	}
</style>
