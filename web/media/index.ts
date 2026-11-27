// The video/voice client module's public surface (ADR 0007). This is the only file the video
// panel in `web/` (owned by @Danny-Defeato) should import from.
//
// Lifecycle: call `createMediaClient` once per app lifetime, not once per panel mount. The
// client subscribes to the browser's `devicechange` event for as long as it exists, and there
// is no `dispose()` to undo that subscription — `leave()` only tears down the current call, not
// the client itself. Creating a new client on every mount leaves one more `devicechange`
// listener behind each time.
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
	TrackAvailability,
	TrackKind
} from './types';
