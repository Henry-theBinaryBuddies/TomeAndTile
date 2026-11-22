// The video/voice client module's public surface (ADR 0007). This is the only file the video
// panel in `web/` (owned by @Danny-Defeato) should import from.
export { createMediaClient, type MediaClient, type MediaClientOptions } from './client';
export type {
	DeviceKind,
	DeviceLists,
	MediaClientState,
	MediaDeviceOption,
	MediaFailure,
	MediaFailureKind,
	MediaStatus,
	ParticipantInfo,
	SelectedDeviceIds,
	TrackKind
} from './types';
