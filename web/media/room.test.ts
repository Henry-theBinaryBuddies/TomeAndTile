import { describe, expect, it, vi } from 'vitest';

const getLocalDevices = vi.fn(async () => []);

vi.mock('livekit-client', () => ({
	Room: { getLocalDevices },
	// Only what room.ts imports at module scope; this file never constructs a Room.
	RoomEvent: {},
	Track: { Source: { Microphone: 'microphone', Camera: 'camera' } },
	MediaDeviceFailure: { getFailure: () => undefined }
}));

describe('listLocalDevices', () => {
	it('never asks for permission, only lists what the browser already exposes', async () => {
		const { listLocalDevices } = await import('./room');
		await listLocalDevices('audioinput');

		expect(getLocalDevices).toHaveBeenCalledWith('audioinput', false);
	});
});
