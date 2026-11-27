<script lang="ts">
	import type { DeviceKind } from '../../../media/index';
	import {
		DEFAULT_DEVICE_LABEL,
		cameraButtonLabel,
		controlsView,
		deviceLabel,
		failureText,
		getMediaClient,
		joinDisabled,
		joinLabel,
		micButtonLabel,
		statusText
	} from '../video';
	import VideoTile from './VideoTile.svelte';

	// The call belongs to the page, not to this panel, so collapsing the panel does not leave it.
	const media = getMediaClient();
	const { state: callState } = media;

	const controls = $derived(controlsView($callState));
	const inCall = $derived($callState.status === 'joined');

	let busyAction = $state(false);

	async function onJoinLeave() {
		if (joinDisabled($callState.status) || busyAction) return;
		busyAction = true;
		try {
			if ($callState.status === 'joined') await media.leave();
			else await media.join();
		} catch {
			// The module reports its failures in `failure`; nothing else to show here.
		} finally {
			busyAction = false;
		}
	}

	function onMic() {
		if (!controls.enabled) return;
		if ($callState.micEnabled) media.mute();
		else media.unmute();
	}

	async function onCamera() {
		if (!controls.enabled) return;
		try {
			if ($callState.cameraEnabled) media.disableCamera();
			else await media.enableCamera();
		} catch {
			// Reported through `failure`.
		}
	}

	// ---- device pickers ----
	// The media module keeps the device lists current and remembers the choice, even before
	// joining and across leave and join, so the panel only reads them. A switch that fails is
	// reported through `failure`, shown above.
	const mics = $derived($callState.devices.audioinput);
	const cameras = $derived($callState.devices.videoinput);
	const selectedMic = $derived($callState.selectedDeviceId.audioinput ?? '');
	const selectedCamera = $derived($callState.selectedDeviceId.videoinput ?? '');

	function onPick(kind: DeviceKind, event: Event) {
		const deviceId = (event.currentTarget as HTMLSelectElement).value;
		if (deviceId) void media.selectDevice(kind, deviceId);
	}
</script>

<div class="video">
	<div class="bar">
		<button
			type="button"
			class="primary"
			disabled={joinDisabled($callState.status) || busyAction}
			onclick={onJoinLeave}
		>
			{joinLabel($callState.status)}
		</button>
		<button
			type="button"
			aria-pressed={$callState.micEnabled}
			aria-describedby="video-controls-note"
			disabled={!controls.enabled}
			onclick={onMic}>{micButtonLabel($callState.micEnabled)}</button
		>
		<button
			type="button"
			aria-pressed={$callState.cameraEnabled}
			aria-describedby="video-controls-note"
			disabled={!controls.enabled}
			onclick={onCamera}>{cameraButtonLabel($callState.cameraEnabled)}</button
		>
	</div>

	<p class="note" id="video-controls-note">{inCall && controls.enabled ? '' : controls.reason}</p>
	<p class="status" role="status" aria-live="polite">{statusText($callState.status)}</p>
	{#if $callState.failure}
		<p class="error" role="alert">{failureText($callState.failure.kind)}</p>
	{/if}

	{#if inCall && controls.enabled}
		<div class="devices">
			<label>
				<span>Microphone</span>
				<select value={selectedMic} onchange={(e) => onPick('audioinput', e)} disabled={mics.length === 0}>
					<option value="" disabled>{mics.length === 0 ? 'No microphone found' : DEFAULT_DEVICE_LABEL}</option>
					{#each mics as device, i (device.deviceId)}
						<option value={device.deviceId}>{deviceLabel(device.label, i, 'audioinput')}</option>
					{/each}
				</select>
			</label>
			<label>
				<span>Camera</span>
				<select
					value={selectedCamera}
					onchange={(e) => onPick('videoinput', e)}
					disabled={cameras.length === 0}
				>
					<option value="" disabled>{cameras.length === 0 ? 'No camera found' : DEFAULT_DEVICE_LABEL}</option>
					{#each cameras as device, i (device.deviceId)}
						<option value={device.deviceId}>{deviceLabel(device.label, i, 'videoinput')}</option>
					{/each}
				</select>
			</label>
		</div>
	{/if}

	{#if $callState.participants.length > 0}
		<ul class="tiles" aria-label="People in the call">
			{#each $callState.participants as participant (participant.id)}
				<li><VideoTile {participant} {media} call={$callState} /></li>
			{/each}
		</ul>
	{:else}
		<p class="empty">{inCall ? 'Nobody else is here yet.' : 'Video tiles will appear here after you join.'}</p>
	{/if}
</div>

<style>
	.video {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		min-width: 0;
	}
	.bar {
		display: flex;
		flex-wrap: wrap;
		gap: 0.6rem;
	}
	.bar button {
		min-height: 44px;
		padding: 0 0.8rem;
	}
	.note,
	.status,
	.error {
		margin: 0;
		font-size: 0.85rem;
	}
	.note,
	.status {
		color: var(--muted);
	}
	.note:empty,
	.status:empty {
		display: none;
	}
	.error {
		color: #ff9e9e;
	}
	.devices {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}
	.devices label {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		font-size: 0.8rem;
		color: var(--muted);
	}
	.devices select {
		min-height: 44px;
		max-width: 100%;
	}
	.tiles {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(8.5rem, 1fr));
		gap: 0.5rem;
	}
	.empty {
		margin: 0;
		color: var(--muted);
	}
</style>
