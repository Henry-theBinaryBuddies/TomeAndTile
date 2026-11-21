<!--
	GM tool: "Set map image". Picks a file, uploads it to the server, then asks the server
	to put it on the current level (ADR 0014). Nothing changes on screen until the server
	answers with the updated scene, so the control says "waiting" until that arrives.
	The state of the upload lives in a store outside this component, so closing the panel,
	changing docks or switching phone tabs does not lose it: coming back shows the running
	upload or its result. The file input and its label are real elements, so keyboard and
	screen readers work.
-->
<script lang="ts">
	import { ACCEPTED_TYPES, getUploadStore, isBusy, startMapImageUpload } from '../upload';

	const status = getUploadStore().status;

	// One upload at a time: the input is disabled until the server has answered.
	const busy = $derived(isBusy($status));

	function onPick(event: Event): void {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		// Clear the input so choosing the same file again (a retry) still counts as a change.
		input.value = '';
		if (!file || busy) return;
		void startMapImageUpload(file);
	}

	const message = $derived(
		$status.kind === 'uploading'
			? 'Uploading the image…'
			: $status.kind === 'waiting'
				? 'Uploaded. Waiting for the server to update the map…'
				: $status.kind === 'done'
					? 'The map image is updated.'
					: ''
	);
</script>

<!-- aria-busy is on the wrapper, not on the status line: a busy live region is not announced. -->
<div class="tool" aria-busy={busy}>
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
	<p class="tool-status" role="status" aria-live="polite">{message}</p>
	<p class="tool-error" role="alert">{$status.kind === 'error' ? $status.text : ''}</p>
	<p id="map-image-help" class="tool-help">
		PNG, JPEG or WebP, up to 20 MiB. It replaces the map picture for everyone.
	</p>
</div>
