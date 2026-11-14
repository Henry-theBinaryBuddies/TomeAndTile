<script lang="ts">
	import { layout } from '../layout/store';
	import {
		PANEL_TITLES,
		PLACEMENTS,
		PLACEMENT_LABELS,
		type PanelId,
		type Placement
	} from '../layout/model';
	import { phoneOpen, viewportSize } from '../layout/viewport';

	type Mode = 'docked' | 'floating' | 'sheet';

	let { id, mode }: { id: PanelId; mode: Mode } = $props();

	const HINTS: Record<PanelId, string> = {
		chat: 'Chat and dice rolls will appear here.',
		sheet: 'Your character sheet will appear here.',
		initiative: 'The initiative order will appear here.',
		video: 'Video tiles will appear here.',
		tools: 'GM tools will appear here.'
	};

	const panel = $derived($layout.panels[id]);
	const title = $derived(PANEL_TITLES[id]);
	const collapsed = $derived(mode !== 'sheet' && panel.collapsed);

	const floatStyle = $derived(
		mode === 'floating'
			? `left:${panel.rect.x}px;top:${panel.rect.y}px;width:${panel.rect.w}px;` +
					(collapsed ? '' : `height:${panel.rect.h}px;`)
			: undefined
	);

	function onPlacement(e: Event) {
		layout.setPlacement(id, (e.currentTarget as HTMLSelectElement).value as Placement, $viewportSize);
	}

	// Dragging a floating panel by its grip (mouse, pen or finger).
	let drag: { px: number; py: number; x: number; y: number } | null = null;

	function gripDown(e: PointerEvent) {
		if (e.pointerType === 'mouse' && e.button !== 0) return;
		(e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
		drag = { px: e.clientX, py: e.clientY, x: panel.rect.x, y: panel.rect.y };
	}
	function gripMove(e: PointerEvent) {
		if (!drag) return;
		layout.setFloatPosition(id, drag.x + e.clientX - drag.px, drag.y + e.clientY - drag.py, $viewportSize);
	}
	function gripUp() {
		drag = null;
	}

	// The same grip moves with the arrow keys: 16 px, or 64 px with Shift.
	function gripKey(e: KeyboardEvent) {
		const step = e.shiftKey ? 64 : 16;
		const moves: Record<string, [number, number]> = {
			ArrowLeft: [-step, 0],
			ArrowRight: [step, 0],
			ArrowUp: [0, -step],
			ArrowDown: [0, step]
		};
		const move = moves[e.key];
		if (!move) return;
		e.preventDefault();
		layout.moveFloat(id, move[0], move[1], $viewportSize);
	}
</script>

<section
	class="panel {mode}"
	class:collapsed
	style={floatStyle}
	data-panel={id}
	aria-labelledby="panel-{id}-title"
>
	<header class="panel-head">
		{#if mode === 'floating'}
			<button
				type="button"
				class="icon grip"
				aria-label="Move {title}. Use the arrow keys, or hold Shift to move faster."
				title="Drag to move"
				onpointerdown={gripDown}
				onpointermove={gripMove}
				onpointerup={gripUp}
				onpointercancel={gripUp}
				onkeydown={gripKey}>⠿</button
			>
		{/if}
		<h2 id="panel-{id}-title">{title}</h2>
		{#if mode === 'sheet'}
			<button type="button" class="icon" aria-label="Close {title}" onclick={() => phoneOpen.set(null)}
				>✕</button
			>
		{:else}
			<select aria-label="{title} position" value={panel.placement} onchange={onPlacement}>
				{#each PLACEMENTS as placement (placement)}
					<option value={placement}>{PLACEMENT_LABELS[placement]}</option>
				{/each}
			</select>
			<button
				type="button"
				class="icon"
				aria-expanded={!collapsed}
				aria-controls="panel-{id}-body"
				aria-label="{collapsed ? 'Expand' : 'Collapse'} {title}"
				onclick={() => layout.toggleCollapsed(id)}>{collapsed ? '▸' : '▾'}</button
			>
		{/if}
	</header>
	{#if !collapsed}
		<div class="panel-body" id="panel-{id}-body">
			<p class="empty">{HINTS[id]}</p>
		</div>
	{/if}
</section>
