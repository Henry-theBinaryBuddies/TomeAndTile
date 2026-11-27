<script lang="ts">
	import {
		AUDIO_BLOCKED_BUTTON,
		AUDIO_BLOCKED_TEXT,
		audioParticipants,
		audioTrackParams,
		getMediaClient,
		track
	} from '../video';

	// Always mounted (in the shell), so other people stay audible when the Video panel is
	// collapsed or closed. Only remote participants get an element: your own microphone is
	// never played back to you.
	const media = getMediaClient();
	const { state: callState } = media;
	const remote = $derived(audioParticipants($callState.participants));
</script>

<div class="audio" aria-hidden="true">
	{#each remote as participant (participant.id)}
		<!-- svelte-ignore a11y_media_has_caption -->
		<audio autoplay use:track={audioTrackParams(media, participant)}></audio>
	{/each}
</div>

<!-- Browsers block sound until the person does something. The prompt lives here, not in the
     Video panel, so it is still there when that panel is collapsed or closed. -->
{#if $callState.audioPlaybackBlocked}
	<div class="blocked" role="status">
		<span>{AUDIO_BLOCKED_TEXT}</span>
		<button type="button" onclick={() => void media.startAudioPlayback()}>{AUDIO_BLOCKED_BUTTON}</button>
	</div>
{/if}

<style>
	.audio {
		display: none;
	}
	.blocked {
		position: fixed;
		left: 50%;
		bottom: calc(var(--tabs-h) + 0.5rem);
		transform: translateX(-50%);
		z-index: 50;
		display: flex;
		align-items: center;
		gap: 0.6rem;
		max-width: calc(100vw - 1rem);
		padding: 0.4rem 0.6rem;
		background: var(--panel-head);
		border: 1px solid var(--accent);
		border-radius: 8px;
		font-size: 0.85rem;
	}
	.blocked button {
		min-height: 44px;
		padding: 0 0.8rem;
	}
</style>
