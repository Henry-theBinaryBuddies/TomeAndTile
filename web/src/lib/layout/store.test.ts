import { get } from 'svelte/store';
import { describe, expect, it } from 'vitest';
import { LAYOUT_STORAGE_KEY, createLayoutStore } from './store';

const vp = { width: 1000, height: 700 };

function fakeStorage(initial?: string) {
	const data = new Map<string, string>();
	if (initial !== undefined) data.set(LAYOUT_STORAGE_KEY, initial);
	return {
		data,
		getItem: (k: string) => data.get(k) ?? null,
		setItem: (k: string, v: string) => void data.set(k, v)
	};
}

describe('layout store', () => {
	it('starts from the defaults when nothing is saved', () => {
		const store = createLayoutStore(fakeStorage());
		expect(get(store).panels.chat.placement).toBe('right');
	});

	it('saves every change and restores it in a new store', () => {
		const storage = fakeStorage();
		const a = createLayoutStore(storage);
		a.dock('chat', 'left');
		a.toggleCollapsed('video');
		a.setPlacement('tools', 'floating', vp);
		const b = createLayoutStore(storage);
		expect(get(b).panels.chat.placement).toBe('left');
		expect(get(b).panels.video.collapsed).toBe(true);
		expect(get(b).panels.tools.placement).toBe('floating');
	});

	it('ignores corrupt saved data', () => {
		const store = createLayoutStore(fakeStorage('{not json'));
		expect(get(store).panels.sheet.placement).toBe('bottom');
	});

	it('survives a storage that throws', () => {
		const broken = {
			getItem: () => {
				throw new Error('blocked');
			},
			setItem: () => {
				throw new Error('full');
			}
		};
		const store = createLayoutStore(broken);
		expect(() => store.toggleCollapsed('chat')).not.toThrow();
		expect(get(store).panels.chat.collapsed).toBe(true);
	});

	it('reset restores the defaults', () => {
		const store = createLayoutStore(fakeStorage());
		store.dock('chat', 'left');
		store.reset();
		expect(get(store).panels.chat.placement).toBe('right');
	});
});
