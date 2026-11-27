// The one video/voice client the page shares (ADR 0007). The panel code reaches `web/media`
// only through its index, never `livekit-client`. Creating the client asks for no camera or
// microphone and opens no connection: that happens only when the user presses Join.
import { createMediaClient, type MediaClient } from '../../../media/index';
import { getGameClient } from '../net';

let instance: MediaClient | null = null;

/**
 * The page's media client, created on first use. It lives outside any panel on purpose:
 * collapsing or closing the Video panel unmounts its body, and that must not end the call.
 */
export function getMediaClient(): MediaClient {
	if (!instance) {
		instance = createMediaClient({ requestToken: () => getGameClient().requestMediaToken() });
	}
	return instance;
}
