<script lang="ts">
	import type { MediaClient, ParticipantInfo } from '../../../media/index';
	import { tileView, track } from '../video';

	let { participant, media }: { participant: ParticipantInfo; media: MediaClient } = $props();

	const view = $derived(tileView(participant));
</script>

<article class="tile" aria-label={view.summary}>
	<div class="picture">
		<!-- Video only: sound is played by the always-present audio elements, never here. -->
		<!-- svelte-ignore a11y_media_has_caption -->
		<video
			muted
			playsinline
			aria-hidden="true"
			class:hidden={!participant.cameraEnabled}
			use:track={{ media, participantId: participant.id, kind: 'video', version: participant.cameraEnabled }}
		></video>
		{#if !participant.cameraEnabled}<span class="off" aria-hidden="true">Camera off</span>{/if}
	</div>
	<p class="name" aria-hidden="true">{view.name}</p>
	<p class="state" aria-hidden="true">{view.micText}</p>
</article>

<style>
	.tile {
		background: var(--panel-head);
		border: 1px solid var(--border);
		border-radius: 8px;
		overflow: hidden;
	}
	.picture {
		position: relative;
		aspect-ratio: 4 / 3;
		background: #0e1016;
		display: grid;
		place-items: center;
	}
	video {
		width: 100%;
		height: 100%;
		object-fit: cover;
	}
	video.hidden {
		display: none;
	}
	.off {
		color: var(--muted);
		font-size: 0.8rem;
	}
	.name,
	.state {
		margin: 0;
		padding: 0.15rem 0.5rem;
		font-size: 0.8rem;
		overflow-wrap: anywhere;
	}
	.name {
		font-weight: 600;
		padding-top: 0.35rem;
	}
	.state {
		color: var(--muted);
		padding-bottom: 0.35rem;
	}
</style>
