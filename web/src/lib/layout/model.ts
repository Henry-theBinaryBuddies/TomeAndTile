// Pure layout logic for the panel shell. No Svelte, no DOM, so it is fully
// unit-testable. Every function returns a new state and never mutates its input.
// The store (store.ts) wraps these; the panel components never touch this directly.

export const PANEL_IDS = ['chat', 'sheet', 'initiative', 'video', 'tools'] as const;
export type PanelId = (typeof PANEL_IDS)[number];

export const EDGES = ['left', 'right', 'top', 'bottom'] as const;
export type Edge = (typeof EDGES)[number];

export const PLACEMENTS = [...EDGES, 'floating'] as const;
export type Placement = (typeof PLACEMENTS)[number];

export const PANEL_TITLES: Record<PanelId, string> = {
	chat: 'Chat',
	sheet: 'Sheet',
	initiative: 'Initiative',
	video: 'Video',
	tools: 'Tools'
};

export const PLACEMENT_LABELS: Record<Placement, string> = {
	left: 'Dock left',
	right: 'Dock right',
	top: 'Dock top',
	bottom: 'Dock bottom',
	floating: 'Floating'
};

export interface Rect {
	x: number;
	y: number;
	w: number;
	h: number;
}

export interface PanelState {
	placement: Placement;
	collapsed: boolean;
	/** Position and size used while floating. Kept when docked so floating again restores it. */
	rect: Rect;
}

export interface LayoutState {
	panels: Record<PanelId, PanelState>;
	/** Stacking order: position inside a docked edge, and which floating panel is on top (last). */
	order: PanelId[];
}

export interface Viewport {
	width: number;
	height: number;
}

export const FLOAT_SIZE = { w: 320, h: 360 } as const;
export const MIN_FLOAT = { w: 200, h: 120 } as const;

const DEFAULT_PLACEMENT: Record<PanelId, Edge> = {
	chat: 'right',
	video: 'right',
	initiative: 'top',
	sheet: 'bottom',
	tools: 'bottom'
};

export function defaultLayout(): LayoutState {
	const panels = {} as Record<PanelId, PanelState>;
	PANEL_IDS.forEach((id, i) => {
		panels[id] = {
			placement: DEFAULT_PLACEMENT[id],
			collapsed: false,
			rect: { x: 96 + i * 32, y: 96 + i * 32, ...FLOAT_SIZE }
		};
	});
	return { panels, order: [...PANEL_IDS] };
}

function clamp(n: number, lo: number, hi: number): number {
	return Math.min(Math.max(n, lo), Math.max(lo, hi));
}

/** Keep a floating rectangle fully inside the viewport. */
export function clampRect(rect: Rect, vp: Viewport): Rect {
	const w = Math.min(Math.max(rect.w, MIN_FLOAT.w), vp.width);
	const h = Math.min(Math.max(rect.h, MIN_FLOAT.h), vp.height);
	return {
		w,
		h,
		x: clamp(rect.x, 0, vp.width - w),
		y: clamp(rect.y, 0, vp.height - h)
	};
}

function sameRect(a: Rect, b: Rect): boolean {
	return a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
}

function patchPanel(state: LayoutState, id: PanelId, patch: Partial<PanelState>): LayoutState {
	return { ...state, panels: { ...state.panels, [id]: { ...state.panels[id], ...patch } } };
}

function bringToEnd(order: PanelId[], id: PanelId): PanelId[] {
	return [...order.filter((p) => p !== id), id];
}

export function dockPanel(state: LayoutState, id: PanelId, edge: Edge): LayoutState {
	return { ...patchPanel(state, id, { placement: edge }), order: bringToEnd(state.order, id) };
}

export function floatPanel(state: LayoutState, id: PanelId, vp: Viewport): LayoutState {
	const next = patchPanel(state, id, {
		placement: 'floating',
		rect: clampRect(state.panels[id].rect, vp)
	});
	return { ...next, order: bringToEnd(state.order, id) };
}

export function setPlacement(
	state: LayoutState,
	id: PanelId,
	placement: Placement,
	vp: Viewport
): LayoutState {
	return placement === 'floating' ? floatPanel(state, id, vp) : dockPanel(state, id, placement);
}

export function setFloatPosition(
	state: LayoutState,
	id: PanelId,
	x: number,
	y: number,
	vp: Viewport
): LayoutState {
	const panel = state.panels[id];
	if (panel.placement !== 'floating') return state;
	const rect = clampRect({ ...panel.rect, x, y }, vp);
	return sameRect(rect, panel.rect) ? state : patchPanel(state, id, { rect });
}

export function moveFloat(
	state: LayoutState,
	id: PanelId,
	dx: number,
	dy: number,
	vp: Viewport
): LayoutState {
	const { rect } = state.panels[id];
	return setFloatPosition(state, id, rect.x + dx, rect.y + dy, vp);
}

export function toggleCollapsed(state: LayoutState, id: PanelId): LayoutState {
	return patchPanel(state, id, { collapsed: !state.panels[id].collapsed });
}

/** Re-clamp every floating panel after the window changes size. Returns the same object if nothing moved. */
export function clampAll(state: LayoutState, vp: Viewport): LayoutState {
	let changed = false;
	const panels = { ...state.panels };
	for (const id of PANEL_IDS) {
		const panel = panels[id];
		if (panel.placement !== 'floating') continue;
		const rect = clampRect(panel.rect, vp);
		if (!sameRect(rect, panel.rect)) {
			panels[id] = { ...panel, rect };
			changed = true;
		}
	}
	return changed ? { ...state, panels } : state;
}

export function panelsAt(state: LayoutState, edge: Edge): PanelId[] {
	return state.order.filter((id) => state.panels[id].placement === edge);
}

export function floatingPanels(state: LayoutState): PanelId[] {
	return state.order.filter((id) => state.panels[id].placement === 'floating');
}

// ---------------------------------------------------------------- persistence

function isRecord(v: unknown): v is Record<string, unknown> {
	return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function isRect(v: unknown): v is Rect {
	return (
		isRecord(v) &&
		['x', 'y', 'w', 'h'].every((k) => typeof v[k] === 'number' && Number.isFinite(v[k] as number))
	);
}

/** Turn untrusted saved JSON back into a valid layout, falling back to defaults field by field. */
export function parseLayout(value: unknown): LayoutState {
	const base = defaultLayout();
	if (!isRecord(value) || !isRecord(value.panels)) return base;
	for (const id of PANEL_IDS) {
		const saved = value.panels[id];
		if (!isRecord(saved)) continue;
		const placement = PLACEMENTS.find((p) => p === saved.placement);
		base.panels[id] = {
			placement: placement ?? base.panels[id].placement,
			collapsed: typeof saved.collapsed === 'boolean' ? saved.collapsed : false,
			rect: isRect(saved.rect) ? { ...saved.rect } : base.panels[id].rect
		};
	}
	if (Array.isArray(value.order)) {
		const seen = new Set<PanelId>();
		for (const id of value.order) {
			const known = PANEL_IDS.find((p) => p === id);
			if (known) seen.add(known);
		}
		base.order = [...seen, ...PANEL_IDS.filter((id) => !seen.has(id))];
	}
	return base;
}
