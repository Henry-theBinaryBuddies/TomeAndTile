// The one place layout state lives (docked edge, collapsed, floating position).
// Panel components read it and call these actions; they hold no layout state of
// their own, so a docking library can replace the buttons later without touching them.
import { writable, type Readable } from 'svelte/store';
import * as model from './model';
import type { Edge, LayoutState, PanelId, Placement, Viewport } from './model';

export const LAYOUT_STORAGE_KEY = 'tt:layout:v1';

type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

export interface LayoutStore extends Readable<LayoutState> {
	setPlacement(id: PanelId, placement: Placement, vp: Viewport): void;
	dock(id: PanelId, edge: Edge): void;
	setFloatPosition(id: PanelId, x: number, y: number, vp: Viewport): void;
	moveFloat(id: PanelId, dx: number, dy: number, vp: Viewport): void;
	toggleCollapsed(id: PanelId): void;
	clamp(vp: Viewport): void;
	reset(): void;
}

function load(storage: Storage | undefined): LayoutState {
	try {
		const raw = storage?.getItem(LAYOUT_STORAGE_KEY);
		return raw ? model.parseLayout(JSON.parse(raw)) : model.defaultLayout();
	} catch {
		return model.defaultLayout();
	}
}

export function createLayoutStore(storage?: Storage): LayoutStore {
	const { subscribe, update, set } = writable<LayoutState>(load(storage));
	subscribe((state) => {
		try {
			storage?.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(state));
		} catch {
			// Private mode or a full disk: the layout simply is not remembered.
		}
	});
	return {
		subscribe,
		setPlacement: (id, placement, vp) => update((s) => model.setPlacement(s, id, placement, vp)),
		dock: (id, edge) => update((s) => model.dockPanel(s, id, edge)),
		setFloatPosition: (id, x, y, vp) => update((s) => model.setFloatPosition(s, id, x, y, vp)),
		moveFloat: (id, dx, dy, vp) => update((s) => model.moveFloat(s, id, dx, dy, vp)),
		toggleCollapsed: (id) => update((s) => model.toggleCollapsed(s, id)),
		clamp: (vp) => update((s) => model.clampAll(s, vp)),
		reset: () => set(model.defaultLayout())
	};
}

export const layout = createLayoutStore(typeof localStorage === 'undefined' ? undefined : localStorage);
