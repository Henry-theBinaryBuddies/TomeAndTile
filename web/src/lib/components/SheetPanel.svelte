<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { RollMode } from '@tomeandtile/contracts';
	import { getGameClient } from '../net';
	import { rejectionText } from '../chat';
	import {
		abilityGroups,
		actingTokenId,
		beginTargeting,
		buildUseAbilityKind,
		endTargeting,
		keepTargeting,
		limitWarning,
		pickCount,
		pickToken,
		picksTargets,
		removePick,
		setMode,
		sheetChoices,
		startTargeting,
		statBlockView,
		targetingSession,
		updateTargeting,
		type AbilityView
	} from '../sheet';

	const client = getGameClient();
	const { state: panelState, lastRejection } = client;

	// ---- whose sheet ----
	const choices = $derived(sheetChoices($panelState));
	let selectedId = $state('');
	const current = $derived(choices.find((c) => c.creatureId === selectedId) ?? choices[0]);
	// A plain string: `current` is rebuilt on every server update, but the id only changes when
	// another character is shown. Effects that must ignore unrelated updates read this.
	const currentId = $derived(current?.creatureId ?? '');
	// Which token acts, when the character has more than one on this map.
	let chosenTokenId = $state('');
	const actingId = $derived(current ? actingTokenId(current, chosenTokenId) : null);
	const creature = $derived(current ? $panelState.creatures[current.creatureId] : undefined);
	const block = $derived(creature ? statBlockView(creature) : null);
	const groups = $derived(creature ? abilityGroups(creature) : []);
	const canAct = $derived(actingId !== null);

	// ---- targeting ----
	// Kept in a shared store, not here: the map adds picks too, and on a phone this panel is
	// closed (unmounted) while the person taps the map.
	const targeting = $derived($targetingSession?.state ?? null);
	let startedFrom = '';
	let wasTargeting = false;
	let sentId = $state<string | null>(null);
	let sentName = $state('');
	let sendNote = $state('');
	let targetingEl = $state<HTMLElement>();

	const sceneTokens = $derived(
		Object.values($panelState.tokens).filter(
			(t) => $panelState.activeSceneId === '' || t.sceneId === $panelState.activeSceneId
		)
	);
	const warning = $derived(targeting ? limitWarning(targeting) : '');
	const rejection = $derived(
		$lastRejection && sentId && $lastRejection.clientIntentId === sentId
			? rejectionText($lastRejection.code)
			: ''
	);

	function tokenName(id: string): string {
		return $panelState.tokens[id]?.name.trim() || 'Unknown';
	}

	async function start(ability: AbilityView) {
		if (!current) return;
		beginTargeting(currentId, startTargeting(ability));
		startedFrom = `ability-${ability.abilityId}`;
		sentId = null;
		sendNote = '';
		await tick();
		targetingEl?.focus();
	}

	function stop() {
		endTargeting();
	}

	// However picking ended (Confirm, Cancel, Escape, another character), put focus back on the
	// ability's button if this panel is on screen.
	$effect(() => {
		const active = targeting !== null;
		if (wasTargeting && !active) {
			void tick().then(() => document.getElementById(startedFrom)?.focus());
		}
		wasTargeting = active;
	});

	function confirm() {
		if (!targeting || !actingId) return;
		const id = client.send(buildUseAbilityKind(actingId, targeting));
		if (id === null) {
			sendNote = 'Not connected, so that was not sent.';
			return;
		}
		sendNote = '';
		sentId = id;
		sentName = targeting.abilityName;
		stop();
	}

	// Escape is handled for the whole page by the map (the Sheet may be closed), and it runs
	// before the phone layout's own Escape handler, so the first Escape only ends picking.

	// Leave targeting if the person switches character, or the character goes away. Reads only
	// the id, so an unrelated server update (a roll, a move) does not end it mid-pick.
	$effect(() => {
		const shown = currentId;
		untrack(() => {
			if ($targetingSession && !keepTargeting($targetingSession.creatureId, shown)) endTargeting();
		});
	});

	// A different character has different tokens: forget the choice.
	$effect(() => {
		void currentId;
		untrack(() => {
			chosenTokenId = '';
		});
	});

	const MODES: [RollMode, string][] = [
		[RollMode.NORMAL, 'Normal'],
		[RollMode.ADVANTAGE, 'Advantage'],
		[RollMode.DISADVANTAGE, 'Disadvantage']
	];
</script>

{#if !$panelState.joined}
	<p class="empty">Connecting…</p>
{:else if !current || !creature || !block}
	<p class="empty">No character sheet to show yet.</p>
{:else}
	<div class="sheet">
		<header class="who">
			{#if choices.length > 1}
				<label class="picker">
					<span>Character</span>
					<select
						value={current.creatureId}
						onchange={(e) => (selectedId = e.currentTarget.value)}
					>
						{#each choices as choice (choice.creatureId)}
							<option value={choice.creatureId}>{choice.name}</option>
						{/each}
					</select>
				</label>
			{:else}
				<h3 class="name">{block.name}</h3>
			{/if}
			{#if block.summary}<p class="summary">{block.summary}</p>{/if}
		</header>

		{#if current.tokens.length > 1}
			<label class="picker">
				<span>Act as</span>
				<select
					value={chosenTokenId}
					onchange={(e) => (chosenTokenId = e.currentTarget.value)}
					disabled={targeting !== null}
				>
					<option value="">Choose a token</option>
					{#each current.tokens as token (token.tokenId)}
						<option value={token.tokenId}>{token.name}</option>
					{/each}
				</select>
			</label>
			{#if !canAct}
				<p class="note">This character has more than one token on the map. Choose which one acts.</p>
			{/if}
		{/if}

		{#if targeting}
			<section
				class="targeting"
				aria-labelledby="targeting-title"
				tabindex="-1"
				bind:this={targetingEl}
			>
				<h3 id="targeting-title">{targeting.abilityName}</h3>

				{#if picksTargets(targeting)}
					<p class="hint">
						{targeting.ordered
							? `Pick a target for each attack, in order. The same target can be picked again.`
							: 'Pick one or more targets.'}
						{targeting.targetText ? `${targeting.targetText}.` : ''}
					</p>
					<ul class="tokens" aria-label="Tokens on this map">
						{#each sceneTokens as token (token.tokenId)}
							{@const count = pickCount(targeting, token.tokenId)}
							<li>
								<button
									type="button"
									aria-pressed={targeting.ordered ? undefined : count > 0}
									class:picked={count > 0}
									onclick={() => updateTargeting((t) => pickToken(t, token.tokenId))}
								>
									{token.name.trim() || 'Unknown'}{token.tokenId === actingId ? ' (you)' : ''}{targeting.ordered && count > 0 ? ` ×${count}` : ''}
								</button>
							</li>
						{:else}
							<li class="empty">There are no tokens on this map.</li>
						{/each}
					</ul>
					<ol class="picks" aria-label="Chosen targets, in order">
						{#each targeting.picks as id, i (i)}
							<li>
								<span>{targeting.ordered ? `Attack ${i + 1}: ` : ''}{tokenName(id)}</span>
								<button
									type="button"
									class="small"
									aria-label="Remove {tokenName(id)}, pick {i + 1}"
									onclick={() => updateTargeting((t) => removePick(t, i))}>Remove</button
								>
							</li>
						{/each}
					</ol>
				{:else}
					<p class="hint">This ability is used on the character itself.</p>
				{/if}

				<fieldset class="mode">
					<legend>Roll mode</legend>
					{#each MODES as [value, label] (value)}
						<label>
							<input
								type="radio"
								name="roll-mode"
								checked={targeting.mode === value}
								onchange={() => updateTargeting((t) => setMode(t, value))}
							/>
							<span>{label}</span>
						</label>
					{/each}
				</fieldset>

				<p class="warn" role="status" aria-live="polite">{warning}</p>

				<div class="actions">
					<button type="button" class="primary" onclick={confirm}>Confirm</button>
					<button type="button" onclick={stop}>Cancel</button>
				</div>
				<p class="hint small-print">Press Escape to cancel.</p>
			</section>
		{/if}

		<p class="status" role="status" aria-live="polite">
			{sentId && !rejection ? `Sent ${sentName} to the server.` : ''}
		</p>
		{#if sendNote || rejection}
			<p class="error" role="alert">{sendNote || rejection}</p>
		{/if}

		<dl class="stats">
			{#each block.stats as stat (stat.label)}
				<div><dt>{stat.label}</dt><dd>{stat.value}</dd></div>
			{/each}
		</dl>
		{#if block.scores}
			<dl class="stats scores" aria-label="Ability scores">
				{#each block.scores as s (s.label)}
					<div><dt>{s.label}</dt><dd>{s.value}</dd></div>
				{/each}
			</dl>
		{:else}
			<p class="note">Ability scores: unknown.</p>
		{/if}
		{#each block.lists as list (list.label)}
			<p class="line"><strong>{list.label}:</strong> {list.value}</p>
		{/each}
		{#if block.resources.length > 0}
			<p class="line">
				<strong>Resources:</strong>
				{block.resources.map((r) => `${r.label} ${r.value}`).join(', ')}
			</p>
		{/if}

		{#if current.tokens.length === 0}
			<p class="note">This character has no token on the map yet, so its abilities cannot be used.</p>
		{/if}

		{#each groups as group (group.cost)}
			<section class="group" aria-labelledby="group-{group.cost}">
				<h3 id="group-{group.cost}">{group.label}</h3>
				<ul class="abilities">
					{#each group.abilities as ability (ability.abilityId)}
						<li class="ability">
							{#if ability.kind === 'usable' || ability.kind === 'area' || ability.kind === 'unsupported'}
								<button
									type="button"
									id="ability-{ability.abilityId}"
									class="use"
									disabled={ability.kind !== 'usable' || !canAct || targeting !== null}
									aria-describedby="ability-{ability.abilityId}-info"
									onclick={() => void start(ability)}>{ability.name}</button
								>
							{:else}
								<span class="plain">{ability.name}</span>
							{/if}
							<p class="info" id="ability-{ability.abilityId}-info">
								{[
									ability.rangeText !== 'Unknown' ? ability.rangeText : '',
									ability.targetText,
									ability.resourceText,
									ability.recharging ? 'Recharging' : '',
									ability.note,
									ability.kind === 'passive' ? 'Always on.' : '',
									ability.kind === 'no_steps' ? 'Nothing to roll.' : ''
								]
									.filter(Boolean)
									.join(' · ')}
							</p>
							{#each ability.stepLines as line, i (i)}
								<p class="info step">{line}</p>
							{/each}
							{#if ability.description}
								<details>
									<summary>Details</summary>
									<p class="desc">{ability.description}</p>
								</details>
							{/if}
						</li>
					{/each}
				</ul>
			</section>
		{/each}
	</div>
{/if}

<style>
	.sheet {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		min-width: 0;
	}
	.who {
		display: flex;
		flex-wrap: wrap;
		gap: 0.2rem 0.8rem;
		align-items: baseline;
	}
	.name {
		margin: 0;
		font-size: 1.1rem;
		overflow-wrap: anywhere;
	}
	.summary {
		margin: 0;
		color: var(--muted);
		overflow-wrap: anywhere;
	}
	.picker {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		font-size: 0.8rem;
		color: var(--muted);
	}
	.picker select {
		min-height: 44px;
		max-width: 100%;
	}
	.stats {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		margin: 0;
	}
	.stats div {
		min-width: 4.2rem;
		padding: 0.3rem 0.55rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 8px;
		text-align: center;
	}
	.stats dt {
		color: var(--muted);
		font-size: 0.75rem;
	}
	.stats dd {
		margin: 0;
		font-weight: 600;
	}
	.line,
	.note,
	.hint,
	.status,
	.error,
	.warn {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.note,
	.hint,
	.status {
		color: var(--muted);
		font-size: 0.85rem;
	}
	.status:empty,
	.warn:empty {
		display: none;
	}
	.error {
		color: #ff9e9e;
	}
	.warn {
		color: #ffd27a;
		font-size: 0.85rem;
	}
	.group h3,
	.targeting h3 {
		margin: 0.4rem 0 0.2rem;
		font-size: 0.95rem;
	}
	.abilities {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}
	.ability {
		padding: 0.4rem 0.55rem;
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 8px;
		overflow-wrap: anywhere;
	}
	.use {
		min-height: 44px;
		padding: 0 0.8rem;
		font-weight: 600;
	}
	.plain {
		font-weight: 600;
	}
	.info {
		margin: 0.2rem 0 0;
		color: var(--muted);
		font-size: 0.85rem;
	}
	.info:empty {
		display: none;
	}
	.step {
		color: var(--text);
	}
	details {
		margin-top: 0.2rem;
		font-size: 0.85rem;
	}
	summary {
		cursor: pointer;
		color: var(--muted);
		min-height: 24px;
	}
	.desc {
		margin: 0.2rem 0 0;
	}
	.targeting {
		padding: 0.6rem;
		background: var(--panel-head);
		border: 1px solid var(--accent);
		border-radius: 8px;
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}
	.targeting h3 {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.tokens,
	.picks {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}
	.tokens button {
		min-height: 44px;
		padding: 0 0.8rem;
		max-width: 100%;
		overflow-wrap: anywhere;
	}
	.tokens button.picked {
		border-color: var(--accent);
		box-shadow: inset 0 0 0 1px var(--accent);
	}
	.picks li {
		display: flex;
		gap: 0.4rem;
		align-items: center;
		padding: 0.15rem 0.2rem 0.15rem 0.6rem;
		border: 1px solid var(--border);
		border-radius: 999px;
		max-width: 100%;
		overflow-wrap: anywhere;
	}
	.small {
		min-height: 32px;
	}
	.mode {
		display: flex;
		flex-wrap: wrap;
		gap: 0.3rem 0.9rem;
		border: 1px solid var(--border);
		border-radius: 8px;
		margin: 0;
		padding: 0.3rem 0.6rem;
	}
	.mode legend {
		color: var(--muted);
		font-size: 0.8rem;
		padding: 0 0.3rem;
	}
	.mode label {
		display: flex;
		gap: 0.3rem;
		align-items: center;
		min-height: 44px;
	}
	.actions {
		display: flex;
		gap: 0.5rem;
	}
	.actions button {
		min-height: 44px;
		padding: 0 1rem;
	}
	.small-print {
		font-size: 0.75rem;
	}
</style>
