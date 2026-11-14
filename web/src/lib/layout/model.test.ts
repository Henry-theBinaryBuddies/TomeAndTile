import { describe, expect, it } from 'vitest';
import {
	PANEL_IDS,
	clampAll,
	clampRect,
	defaultLayout,
	dockPanel,
	floatPanel,
	floatingPanels,
	moveFloat,
	panelsAt,
	parseLayout,
	setFloatPosition,
	setPlacement,
	toggleCollapsed,
	type LayoutState
} from './model';

const vp = { width: 1000, height: 700 };

describe('defaults', () => {
	it('has the five panels docked in the ADR 0011 positions', () => {
		const s = defaultLayout();
		expect(Object.keys(s.panels).sort()).toEqual([...PANEL_IDS].sort());
		expect(panelsAt(s, 'right')).toEqual(['chat', 'video']);
		expect(panelsAt(s, 'top')).toEqual(['initiative']);
		expect(panelsAt(s, 'bottom')).toEqual(['sheet', 'tools']);
		expect(panelsAt(s, 'left')).toEqual([]);
		expect(floatingPanels(s)).toEqual([]);
		expect(PANEL_IDS.every((id) => !s.panels[id].collapsed)).toBe(true);
	});
});

describe('docking', () => {
	it('moves a panel to another edge and puts it last there', () => {
		const s = dockPanel(defaultLayout(), 'chat', 'left');
		expect(s.panels.chat.placement).toBe('left');
		expect(panelsAt(s, 'left')).toEqual(['chat']);
		expect(panelsAt(s, 'right')).toEqual(['video']);
		const t = dockPanel(s, 'sheet', 'left');
		expect(panelsAt(t, 'left')).toEqual(['chat', 'sheet']);
	});

	it('never mutates its input', () => {
		const before = defaultLayout();
		const snapshot = JSON.stringify(before);
		dockPanel(before, 'chat', 'left');
		toggleCollapsed(before, 'chat');
		floatPanel(before, 'chat', vp);
		expect(JSON.stringify(before)).toBe(snapshot);
	});
});

describe('collapsing', () => {
	it('toggles one panel and leaves the rest alone', () => {
		const s = toggleCollapsed(defaultLayout(), 'video');
		expect(s.panels.video.collapsed).toBe(true);
		expect(s.panels.chat.collapsed).toBe(false);
		expect(toggleCollapsed(s, 'video').panels.video.collapsed).toBe(false);
	});

	it('keeps a collapsed panel collapsed when it moves', () => {
		const s = dockPanel(toggleCollapsed(defaultLayout(), 'chat'), 'chat', 'left');
		expect(s.panels.chat.collapsed).toBe(true);
	});
});

describe('floating', () => {
	it('floats a panel inside the viewport and brings it to the front', () => {
		const s = floatPanel(defaultLayout(), 'sheet', vp);
		expect(s.panels.sheet.placement).toBe('floating');
		expect(floatingPanels(s)).toEqual(['sheet']);
		const t = floatPanel(s, 'tools', vp);
		expect(floatingPanels(t)).toEqual(['sheet', 'tools']);
		const u = floatPanel(t, 'sheet', vp);
		expect(floatingPanels(u)).toEqual(['tools', 'sheet']);
	});

	it('clamps a rectangle that sticks out of the window', () => {
		expect(clampRect({ x: -50, y: 900, w: 320, h: 360 }, vp)).toEqual({ x: 0, y: 340, w: 320, h: 360 });
		expect(clampRect({ x: 900, y: 0, w: 320, h: 360 }, vp).x).toBe(680);
	});

	it('shrinks a panel that is bigger than a small window', () => {
		const r = clampRect({ x: 0, y: 0, w: 320, h: 360 }, { width: 250, height: 200 });
		expect(r).toMatchObject({ w: 250, h: 200, x: 0, y: 0 });
	});

	it('moves with the keyboard step and stops at the window edge', () => {
		let s = floatPanel(defaultLayout(), 'chat', vp);
		const start = s.panels.chat.rect;
		s = moveFloat(s, 'chat', 16, -16, vp);
		expect(s.panels.chat.rect).toMatchObject({ x: start.x + 16, y: start.y - 16 });
		s = moveFloat(s, 'chat', 5000, 5000, vp);
		expect(s.panels.chat.rect).toMatchObject({ x: 680, y: 340 });
	});

	it('ignores a move for a panel that is docked', () => {
		const s = defaultLayout();
		expect(setFloatPosition(s, 'chat', 10, 10, vp)).toBe(s);
	});

	it('remembers the floating spot while docked', () => {
		const floated = setFloatPosition(floatPanel(defaultLayout(), 'chat', vp), 'chat', 200, 150, vp);
		const docked = dockPanel(floated, 'chat', 'left');
		const again = setPlacement(docked, 'chat', 'floating', vp);
		expect(again.panels.chat.rect).toMatchObject({ x: 200, y: 150 });
	});

	it('re-clamps every floating panel when the window shrinks, and is stable otherwise', () => {
		let s = setFloatPosition(floatPanel(defaultLayout(), 'chat', vp), 'chat', 650, 300, vp);
		expect(clampAll(s, vp)).toBe(s);
		s = clampAll(s, { width: 500, height: 400 });
		expect(s.panels.chat.rect.x).toBe(180);
		expect(s.panels.chat.rect.y).toBe(40);
	});
});

describe('saved layouts', () => {
	it('round-trips through JSON', () => {
		let s = dockPanel(defaultLayout(), 'chat', 'left');
		s = toggleCollapsed(s, 'tools');
		s = floatPanel(s, 'video', vp);
		expect(parseLayout(JSON.parse(JSON.stringify(s)))).toEqual(s);
	});

	it('falls back to defaults for garbage', () => {
		for (const bad of [null, 5, 'x', [], {}, { panels: 3 }]) {
			expect(parseLayout(bad)).toEqual(defaultLayout());
		}
	});

	it('repairs bad fields one by one and drops unknown panels', () => {
		const saved = {
			panels: {
				chat: { placement: 'nowhere', collapsed: 'yes', rect: { x: 'a' } },
				video: { placement: 'left', collapsed: true, rect: { x: 1, y: 2, w: 300, h: 300 } },
				ghost: { placement: 'left' }
			},
			order: ['video', 'ghost', 'video', 'chat']
		};
		const s: LayoutState = parseLayout(saved);
		expect(s.panels.chat).toEqual(defaultLayout().panels.chat);
		expect(s.panels.video).toMatchObject({ placement: 'left', collapsed: true });
		expect('ghost' in s.panels).toBe(false);
		expect(s.order.slice(0, 2)).toEqual(['video', 'chat']);
		expect([...s.order].sort()).toEqual([...PANEL_IDS].sort());
	});
});
