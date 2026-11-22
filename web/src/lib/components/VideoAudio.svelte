<script lang="ts">
	import { audioParticipants, getMediaClient, track } from '../video';

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
		<audio autoplay use:track={{ media, participantId: participant.id, kind: 'audio', version: participant }}></audio>
	{/each}
</div>

<style>
	.audio {
		display: none;
	}
</style>
