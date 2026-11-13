//! Message framing helpers that need no browser, so `web/` and the demo page
//! can open a socket without a protobuf library.

use prost::Message as _;
use tt_contracts::v1::{client_message, ClientMessage, Hello};

/// The first frame on a new socket: a `ClientMessage` holding a `Hello`.
/// `last_seq` is 0 (fresh join; the server ignores it today).
pub fn encode_hello(campaign_id: &str, session_token: &str) -> Vec<u8> {
    ClientMessage {
        kind: Some(client_message::Kind::Hello(Hello {
            campaign_id: campaign_id.to_owned(),
            session_token: session_token.to_owned(),
            last_seq: 0,
        })),
    }
    .encode_to_vec()
}
