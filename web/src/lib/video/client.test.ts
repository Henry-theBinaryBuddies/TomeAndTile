import { beforeEach, describe, expect, it, vi } from 'vitest';

const created: { requestToken: () => Promise<unknown> }[] = [];
const requestMediaToken = vi.fn(async () => ({ token: 'secret' }));

vi.mock('../../../media/index', () => ({
	createMediaClient: (options: { requestToken: () => Promise<unknown> }) => {
		created.push(options);
		return { fake: true };
	}
}));
vi.mock('../net', () => ({ getGameClient: () => ({ requestMediaToken }) }));

describe('getMediaClient', () => {
	beforeEach(() => {
		created.length = 0;
		requestMediaToken.mockClear();
		vi.resetModules();
	});

	it('makes one client, and only when first asked for', async () => {
		const { getMediaClient } = await import('./client');
		expect(created).toHaveLength(0);
		const a = getMediaClient();
		const b = getMediaClient();
		expect(a).toBe(b);
		expect(created).toHaveLength(1);
	});

	it('asks for no token until the client itself asks (that is, on Join)', async () => {
		const { getMediaClient } = await import('./client');
		getMediaClient();
		expect(requestMediaToken).not.toHaveBeenCalled();
		await created[0].requestToken();
		expect(requestMediaToken).toHaveBeenCalledTimes(1);
	});
});
