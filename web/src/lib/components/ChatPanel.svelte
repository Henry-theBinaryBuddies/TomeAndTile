<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { Role, RollMode } from '@tomeandtile/contracts';
	import { getGameClient } from '../net';
	import {
		buildChatKind,
		buildRollKind,
		displayName,
		rejectionText,
		resultCardView,
		rollCardView,
		textAfterRejection
	} from '../chat';
	import ResultCard from './ResultCard.svelte';
	import RollCard from './RollCard.svelte';

	const client = getGameClient();
	const { state: panelState, connection, lastRejection } = client;

	const isGm = $derived($panelState.role === Role.GM);
	const status = $derived(
		$connection === 'connecting'
			? 'Connecting…'
			: $connection === 'closed'
				? 'Disconnected. Trying again…'
				: !$panelState.joined
					? 'Joining…'
					: ''
	);

	// ---- the log: chat lines and roll cards, newest at the bottom ----
	let logEl = $state<HTMLElement>();
	let stickToBottom = true;

	function onLogScroll() {
		if (!logEl) return;
		stickToBottom = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 24;
	}

	$effect(() => {
		void $panelState.log.length;
		if (!stickToBottom) return;
		void tick().then(() => {
			if (logEl) logEl.scrollTop = logEl.scrollHeight;
		});
	});

	// ---- chat form ----
	let chatText = $state('');
	let chatNote = $state('');
	let sentChat = $state<{ id: string; text: string } | null>(null);

	function submitChat(e: SubmitEvent) {
		e.preventDefault();
		const kind = buildChatKind(chatText);
		if (!kind) {
			chatNote = 'Type a message first.';
			return;
		}
		const id = client.send(kind);
		if (id === null) {
			chatNote = 'Not connected, so that message was not sent.';
			return;
		}
		chatNote = '';
		sentChat = { id, text: chatText };
		chatText = '';
	}

	// If the server refuses the message just sent and the box is still empty, put the text back.
	$effect(() => {
		const rejection = $lastRejection;
		const sent = sentChat;
		untrack(() => {
			chatText = textAfterRejection(sent, rejection, chatText);
		});
	});

	const chatRejection = $derived(
		$lastRejection && sentChat && $lastRejection.clientIntentId === sentChat.id
			? rejectionText($lastRejection.code)
			: ''
	);

	// ---- roll form ----
	let notation = $state('');
	let mode = $state(RollMode.NORMAL);
	let label = $state('');
	let hidden = $state(false);
	let rollNote = $state('');
	let sentRollId = $state<string | null>(null);

	function submitRoll(e: SubmitEvent) {
		e.preventDefault();
		const kind = buildRollKind({ notation, mode, label, hidden }, isGm);
		if (!kind) {
			rollNote = 'Enter dice to roll, for example d20+5.';
			return;
		}
		const id = client.send(kind);
		if (id === null) {
			rollNote = 'Not connected, so that roll was not sent.';
			return;
		}
		rollNote = '';
		sentRollId = id;
	}

	const rollRejection = $derived(
		$lastRejection && sentRollId && $lastRejection.clientIntentId === sentRollId
			? rejectionText($lastRejection.code)
			: ''
	);
</script>

<div class="chat">
	{#if status}
		<p class="status" role="status">{status}</p>
	{/if}

	<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
	<div
		class="log"
		role="log"
		aria-label="Chat and dice log"
		tabindex="0"
		bind:this={logEl}
		onscroll={onLogScroll}
	>
		{#each $panelState.log as entry (entry.seq)}
			{#if entry.kind === 'chat'}
				<p class="line" class:undone={entry.undone}>
					<span class="who">{displayName($panelState.members, entry.userId)}</span>
					<span class="text">{entry.text}</span>
					{#if entry.undone}<span class="note">(undone)</span>{/if}
				</p>
			{:else if entry.kind === 'result'}
				{@const resolution = $panelState.resolutions[entry.resolutionId]}
				{#if resolution}
					<ResultCard
						view={resultCardView(resolution, $panelState.tokens, $panelState.creatures, entry.warnings)}
						undone={entry.undone}
					/>
				{/if}
			{:else}
				<RollCard
					who={displayName($panelState.members, entry.roll.rollerUserId)}
					view={rollCardView(entry.roll)}
					undone={entry.undone}
				/>
			{/if}
		{:else}
			<p class="empty">No messages yet.</p>
		{/each}
	</div>

	<form class="chat-form" onsubmit={submitChat}>
		<label class="sr-only" for="chat-text">Message</label>
		<input
			id="chat-text"
			type="text"
			autocomplete="off"
			placeholder="Message"
			bind:value={chatText}
		/>
		<button type="submit">Send</button>
	</form>
	{#if chatNote || chatRejection}
		<p class="error" role="alert">{chatNote || chatRejection}</p>
	{/if}

	<form class="roll-form" onsubmit={submitRoll}>
		<label class="dice">
			<span>Dice</span>
			<input type="text" autocomplete="off" placeholder="d20+5" bind:value={notation} />
		</label>
		<label class="mode">
			<span>Mode</span>
			<select bind:value={mode}>
				<option value={RollMode.NORMAL}>Normal</option>
				<option value={RollMode.ADVANTAGE}>Advantage</option>
				<option value={RollMode.DISADVANTAGE}>Disadvantage</option>
			</select>
		</label>
		<label class="label">
			<span>Label</span>
			<input type="text" autocomplete="off" placeholder="Optional" bind:value={label} />
		</label>
		{#if isGm}
			<label class="check">
				<input type="checkbox" bind:checked={hidden} />
				<span>Hide from players</span>
			</label>
		{/if}
		<button type="submit">Roll</button>
	</form>
	{#if rollNote || rollRejection}
		<p class="error" role="alert">{rollNote || rollRejection}</p>
	{/if}
</div>

<style>
	.chat {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		height: 100%;
		min-height: 0;
	}
	.status {
		margin: 0;
		color: var(--muted);
		font-size: 0.85rem;
	}
	.log {
		flex: 1;
		min-height: 4rem;
		overflow: auto;
		background: var(--panel);
		padding: 0.2rem;
		border: 1px solid var(--border);
		border-radius: 8px;
	}
	.line {
		margin: 0.25rem 0;
		overflow-wrap: anywhere;
	}
	.line.undone {
		opacity: 0.6;
	}
	.who {
		margin-right: 0.4rem;
		color: var(--accent);
		font-weight: 600;
	}
	.note {
		color: var(--muted);
		font-size: 0.8rem;
	}
	.empty {
		margin: 0.4rem;
		color: var(--muted);
	}
	.chat-form {
		display: flex;
		gap: 0.4rem;
	}
	.chat-form input {
		flex: 1;
		min-width: 0;
	}
	/* Dice and the Roll button share the first row; the button is last in the page order
	   so keyboard users reach it after every field. */
	.roll-form {
		display: grid;
		grid-template-columns: 1fr 1fr auto;
		grid-template-areas:
			'dice dice go'
			'mode label label'
			'hide hide hide';
		gap: 0.4rem;
		align-items: end;
	}
	.roll-form label {
		display: flex;
		flex-direction: column;
		min-width: 0;
		gap: 0.15rem;
		font-size: 0.8rem;
		color: var(--muted);
	}
	.roll-form label.dice {
		grid-area: dice;
	}
	.roll-form label.mode {
		grid-area: mode;
	}
	.roll-form label.label {
		grid-area: label;
	}
	.roll-form label.check {
		grid-area: hide;
		flex-direction: row;
		align-items: center;
		min-height: var(--control);
	}
	.roll-form button {
		grid-area: go;
	}
	input[type='text'],
	select {
		min-height: var(--control);
		padding: 0 0.5rem;
		font: inherit;
		color: var(--text);
		background: var(--bg);
		border: 1px solid var(--border);
		border-radius: 6px;
	}
	input[type='checkbox'] {
		width: 1.25rem;
		height: 1.25rem;
	}
	button {
		min-height: var(--control);
		padding: 0 0.9rem;
		font: inherit;
		color: var(--text);
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 6px;
		cursor: pointer;
	}
	button:hover,
	input:focus-visible,
	select:focus-visible {
		border-color: var(--accent);
	}
	.error {
		margin: 0;
		color: #ff9a9a;
		font-size: 0.85rem;
	}
</style>
