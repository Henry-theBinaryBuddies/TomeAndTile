<!--
	GM tool: "Set map image". Picks a file, uploads it to the server, then asks the server
	to put it on the current level (ADR 0014). Nothing changes on screen until the server
	answers with the updated scene, so the control says "waiting" until that arrives.
	The file input and its label are real elements, so keyboard and screen readers work.
-->
<script lang="ts">
	import { onDestroy } from 'svelte';
	import { rejectionText } from '../chat';
	import { getGameClient, getUploadSession } from '../net';
	import {
		ACCEPTED_TYPES,
		buildSetBackgroundKind,
		setMapImage,
		setMapImageFailureText
	} from '../upload';

	/** How long to wait for the server's scene update before saying it has not come. */
	const CONFIRM_TIMEOUT_MS = 8000;

	type Status =
		| { kind: 'idle' }
		| { kind: 'uploading' }
		| { kind: 'waiting'; assetId: string; intentId: string }
		| { kind: 'done' }
		| { kind: 'error'; text: string };

	const client = getGameClient();
	const { state: panelState, lastRejection } = client;

	let status = $state<Status>({ kind: 'idle' });
	let confirmTimer: ReturnType<typeof setTimeout> | null = null;
	let gone = false;

	// One upload at a time: the input is disabled until the server has answered.
	const busy = $derived(status.kind === 'uploading' || status.kind === 'waiting');

	function stopTimer(): void {
		if (confirmTimer) clearTimeout(confirmTimer);
		confirmTimer = null;
	}

	function fail(text: string): void {
		stopTimer();
		status = { kind: 'error', text };
	}

	// While waiting: done when the level's background is the uploaded image, failed if the
	// server refused the intent.
	$effect(() => {
		const s = status;
		if (s.kind !== 'waiting') return;
		if ($panelState.mapTarget?.backgroundAssetId === s.assetId) {
			stopTimer();
			status = { kind: 'done' };
			return;
		}
		const rejection = $lastRejection;
		if (rejection && rejection.clientIntentId === s.intentId) {
			fail(rejectionText(rejection.code));
			client.clearRejection();
		}
	});

	async function onPick(event: Event): Promise<void> {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		if (!file || busy) return;
		status = { kind: 'uploading' };
		const session = getUploadSession();
		let result: Awaited<ReturnType<typeof setMapImage>>;
		try {
			result = await setMapImage({
				file,
				target: $panelState.mapTarget,
				socketUrl: session.socketUrl,
				token: session.token,
				sendBackground: (target, assetId) => client.send(buildSetBackgroundKind(target, assetId))
			});
		} catch {
			// A setup mistake, such as a socket address that is not a full URL. Never leave
			// the control stuck on "uploading".
			input.value = '';
			if (!gone) fail('Something went wrong while uploading. Reload the page and try again.');
			return;
		}
		// Clear the input so choosing the same file again (a retry) still counts as a change.
		input.value = '';
		if (gone) return;
		if (!result.ok) {
			fail(setMapImageFailureText(result.reason));
			return;
		}
		status = { kind: 'waiting', assetId: result.asset.assetId, intentId: result.intentId };
		confirmTimer = setTimeout(() => {
			if (status.kind === 'waiting') {
				fail('The server has not confirmed the change yet. Check the map, or try again.');
			}
		}, CONFIRM_TIMEOUT_MS);
	}

	onDestroy(() => {
		gone = true;
		stopTimer();
	});

	const message = $derived(
		status.kind === 'uploading'
			? 'Uploading the image…'
			: status.kind === 'waiting'
				? 'Uploaded. Waiting for the server to update the map…'
				: status.kind === 'done'
					? 'The map image is updated.'
					: ''
	);
</script>

<div class="tool">
	<label class="tool-label" for="map-image-input">Set map image</label>
	<input
		id="map-image-input"
		type="file"
		accept={ACCEPTED_TYPES.join(',')}
		disabled={busy}
		aria-describedby="map-image-help"
		onchange={onPick}
	/>
	<!-- Messages come right after the input: the docked Tools panel is short, and the
	     help text, which matters least, is the part that can be cut off. -->
	<p class="tool-status" role="status" aria-live="polite" aria-busy={busy}>{message}</p>
	<p class="tool-error" role="alert">{status.kind === 'error' ? status.text : ''}</p>
	<p id="map-image-help" class="tool-help">
		PNG, JPEG or WebP, up to 20 MiB. It replaces the map picture for everyone.
	</p>
</div>
