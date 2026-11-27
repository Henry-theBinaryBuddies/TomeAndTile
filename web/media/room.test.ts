import { describe, expect, it, vi } from 'vitest';

const getLocalDevices = vi.fn(async () => []);

// The real string values (livekit-client's RoomEvent is a plain string enum), so a test can
// assert on *which* event a call registered, not just that `room.on` was called at all.
const RoomEventValues = {
	Disconnected: 'disconnected',
	ParticipantConnected: 'participantConnected',
	ParticipantDisconnected: 'participantDisconnected',
	TrackMuted: 'trackMuted',
	TrackUnmuted: 'trackUnmuted',
	TrackPublished: 'trackPublished',
	TrackUnpublished: 'trackUnpublished',
	TrackSubscribed: 'trackSubscribed',
	TrackUnsubscribed: 'trackUnsubscribed',
	LocalTrackPublished: 'localTrackPublished',
	LocalTrackUnpublished: 'localTrackUnpublished',
	ActiveSpeakersChanged: 'activeSpeakersChanged',
	AudioPlaybackStatusChanged: 'audioPlaybackChanged'
} as const;

vi.mock('livekit-client', () => ({
	Room: { getLocalDevices },
	// Only what room.ts imports at module scope; this file never constructs a real Room.
	RoomEvent: RoomEventValues,
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

describe('wrapLiveKitRoom', () => {
	it('onParticipantsChanged registers TrackPublished/TrackUnpublished, not just TrackSubscribed', async () => {
		const { wrapLiveKitRoom } = await import('./room');
		const registeredEvents: string[] = [];
		// A bare stub, not a mock of the whole Room class: onParticipantsChanged only calls
		// `.on`, so that is all this needs to implement.
		const fakeRoom = { on: (event: string) => registeredEvents.push(event) };

		wrapLiveKitRoom(fakeRoom as never).onParticipantsChanged(() => {});

		// Published/Unpublished (a track becoming known at all) is what makes "waiting for
		// video" distinguishable from "camera off"; Subscribed/Unsubscribed alone (already
		// registered before this test was added) only covers the media actually arriving.
		expect(registeredEvents).toContain(RoomEventValues.TrackPublished);
		expect(registeredEvents).toContain(RoomEventValues.TrackUnpublished);
		expect(registeredEvents).toContain(RoomEventValues.TrackSubscribed);
		expect(registeredEvents).toContain(RoomEventValues.TrackUnsubscribed);
	});
});
