import { readable, writable } from 'svelte/store';
import type { PanelId, Viewport } from './model';

/** At this width or less the docked panels become bottom tabs over the map (ADR 0011). */
export const PHONE_QUERY = '(max-width: 640px)';

export const isPhone = readable(false, (set) => {
	if (typeof matchMedia === 'undefined') return;
	const query = matchMedia(PHONE_QUERY);
	set(query.matches);
	const onChange = (e: MediaQueryListEvent) => set(e.matches);
	query.addEventListener('change', onChange);
	return () => query.removeEventListener('change', onChange);
});

export const viewportSize = readable<Viewport>({ width: 1280, height: 800 }, (set) => {
	if (typeof window === 'undefined') return;
	const update = () => set({ width: window.innerWidth, height: window.innerHeight });
	update();
	window.addEventListener('resize', update);
	return () => window.removeEventListener('resize', update);
});

/** Which panel is open as a sheet on the phone layout. UI state only; it is not part of the saved layout. */
export const phoneOpen = writable<PanelId | null>(null);
