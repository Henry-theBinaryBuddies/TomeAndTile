//! LiveKit join tokens (ADR 0007). Pure: no I/O, no clock (time is passed in).
//!
//! A LiveKit access token is a JWT signed with HS256. Claim names follow
//! LiveKit's "Access tokens & grants" reference:
//! <https://docs.livekit.io/frontends/reference/tokens-grants.md>
//! (`iss` = API key, `sub` = participant identity, `nbf`, `exp`, and a
//! `video` grant with `room`, `roomJoin`, `canPublish`, `canSubscribe`,
//! `canPublishData`). That page does not list a participant `name` claim;
//! `name` is emitted because LiveKit's server SDKs set it for the display
//! name, but it is not confirmed by that page.
//!
//! The API secret and every minted token are credentials: nothing here
//! logs, prints or echoes them.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::json;
use sha2::Sha256;
use tt_contracts::v1::{MediaToken, Role};

/// How long a minted token stays valid, in seconds (10 minutes).
pub const TOKEN_TTL_SECS: i64 = 600;

/// Why a [`MediaConfig`] was refused. Never contains the secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaConfigError {
    EmptyApiKey,
    EmptyApiSecret,
    BadUrl,
}

impl fmt::Display for MediaConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::EmptyApiKey => "the LiveKit API key is empty",
            Self::EmptyApiSecret => "the LiveKit API secret is empty",
            Self::BadUrl => "the LiveKit URL must start with wss:// or ws://",
        })
    }
}

impl std::error::Error for MediaConfigError {}

/// Where the SFU is and how to sign for it.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaConfig {
    sfu_url: String,
    api_key: String,
    api_secret: String,
}

impl MediaConfig {
    /// Validate and build. The URL must start with `wss://` or `ws://` and
    /// have something after it; key and secret must be non-empty.
    pub fn new(
        sfu_url: impl Into<String>,
        api_key: impl Into<String>,
        api_secret: impl Into<String>,
    ) -> Result<Self, MediaConfigError> {
        let sfu_url = sfu_url.into();
        let api_key = api_key.into();
        let api_secret = api_secret.into();
        let host = sfu_url
            .strip_prefix("wss://")
            .or_else(|| sfu_url.strip_prefix("ws://"));
        if host.is_none_or(|h| h.is_empty()) {
            return Err(MediaConfigError::BadUrl);
        }
        if api_key.is_empty() {
            return Err(MediaConfigError::EmptyApiKey);
        }
        if api_secret.is_empty() {
            return Err(MediaConfigError::EmptyApiSecret);
        }
        Ok(Self {
            sfu_url,
            api_key,
            api_secret,
        })
    }

    pub fn sfu_url(&self) -> &str {
        &self.sfu_url
    }
}

// By hand so the secret can never reach a log through `{:?}`.
impl fmt::Debug for MediaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MediaConfig")
            .field("sfu_url", &self.sfu_url)
            .field("api_key", &self.api_key)
            .field("api_secret", &"<redacted>")
            .finish()
    }
}

/// The LiveKit room for a campaign: `tt-<campaign_id>`. `None` for an empty
/// id, so no shared room is ever invented.
pub fn room_name(campaign_id: &str) -> Option<String> {
    if campaign_id.is_empty() {
        None
    } else {
        Some(format!("tt-{campaign_id}"))
    }
}

/// Mint a compact HS256 JWT. `ttl_secs` is added to `now_ms` (seconds,
/// rounded down). Saturating, so extreme inputs cannot panic. `None` only if
/// the HMAC key were rejected, which HMAC-SHA256 does not do.
pub fn mint_token(
    config: &MediaConfig,
    room: &str,
    identity: &str,
    display_name: &str,
    can_publish: bool,
    now_ms: i64,
    ttl_secs: i64,
) -> Option<String> {
    let nbf = now_ms.div_euclid(1000);
    let exp = nbf.saturating_add(ttl_secs);
    let header = json!({ "alg": "HS256", "typ": "JWT" });
    let mut claims = json!({
        "iss": config.api_key,
        "sub": identity,
        "nbf": nbf,
        "exp": exp,
        "video": {
            "room": room,
            "roomJoin": true,
            "canPublish": can_publish,
            "canSubscribe": true,
            "canPublishData": can_publish,
        },
    });
    if !display_name.is_empty() {
        claims["name"] = json!(display_name);
    }
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(config.api_secret.as_bytes()).ok()?;
    mac.update(signing_input.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Some(format!("{signing_input}.{signature}"))
}

/// Why no token was issued. `code` is the stable `IntentRejected.code`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDenied {
    /// Media is off, or the campaign has no id to name a room after.
    NotConfigured,
    /// The requester's role gets no token.
    NotPermitted,
}

impl MediaDenied {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotConfigured => "media_not_configured",
            Self::NotPermitted => "not_permitted",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::NotConfigured => "video and voice are not available on this server",
            Self::NotPermitted => "you are not allowed to join the call",
        }
    }
}

/// Who gets what: the whole decision, with no I/O. GM and Player may publish;
/// a Spectator may only subscribe; any other role gets nothing.
pub fn decide(
    config: Option<&MediaConfig>,
    campaign_id: &str,
    user_id: &str,
    display_name: &str,
    role: Role,
    now_ms: i64,
) -> Result<MediaToken, MediaDenied> {
    let config = config.ok_or(MediaDenied::NotConfigured)?;
    let room = room_name(campaign_id).ok_or(MediaDenied::NotConfigured)?;
    let can_publish = match role {
        Role::Gm | Role::Player => true,
        Role::Spectator => false,
        Role::Unspecified => return Err(MediaDenied::NotPermitted),
    };
    if user_id.is_empty() {
        return Err(MediaDenied::NotPermitted);
    }
    let token = mint_token(
        config,
        &room,
        user_id,
        display_name,
        can_publish,
        now_ms,
        TOKEN_TTL_SECS,
    )
    .ok_or(MediaDenied::NotConfigured)?;
    let expires_at_ms = now_ms
        .div_euclid(1000)
        .saturating_add(TOKEN_TTL_SECS)
        .saturating_mul(1000);
    Ok(MediaToken {
        sfu_url: config.sfu_url.clone(),
        token,
        room,
        expires_at_ms,
        can_publish,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const KEY: &str = "test-key";
    const SECRET: &str = "test-secret-not-real";
    const NOW: i64 = 1_700_000_000_123;

    fn cfg() -> MediaConfig {
        MediaConfig::new("wss://sfu.example.test", KEY, SECRET).unwrap()
    }

    fn parts(token: &str) -> Vec<&str> {
        token.split('.').collect()
    }

    fn decode(segment: &str) -> Value {
        let bytes = URL_SAFE_NO_PAD
            .decode(segment)
            .expect("base64url, no padding");
        serde_json::from_slice(&bytes).expect("json")
    }

    fn mint(role: Role, user: &str) -> String {
        decide(Some(&cfg()), "camp", user, "Name", role, NOW)
            .expect("token")
            .token
    }

    fn verifies(token: &str, secret: &str) -> bool {
        let p = parts(token);
        let sig = URL_SAFE_NO_PAD.decode(p[2]).unwrap();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{}.{}", p[0], p[1]).as_bytes());
        mac.verify_slice(&sig).is_ok()
    }

    #[test]
    fn token_has_three_unpadded_base64url_segments() {
        let t = mint(Role::Player, "alice");
        let p = parts(&t);
        assert_eq!(p.len(), 3);
        for s in p {
            assert!(!s.is_empty());
            assert!(s
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        }
    }

    #[test]
    fn header_and_claims_for_each_role() {
        for (role, publish) in [
            (Role::Gm, true),
            (Role::Player, true),
            (Role::Spectator, false),
        ] {
            let t = mint(role, "u1");
            let p = parts(&t);
            assert_eq!(decode(p[0]), json!({"alg": "HS256", "typ": "JWT"}));
            let c = decode(p[1]);
            assert_eq!(c["iss"], KEY);
            assert_eq!(c["sub"], "u1");
            assert_eq!(c["name"], "Name");
            assert_eq!(c["nbf"], 1_700_000_000_i64);
            assert_eq!(c["exp"].as_i64().unwrap() - c["nbf"].as_i64().unwrap(), 600);
            assert_eq!(
                c["video"],
                json!({
                    "room": "tt-camp",
                    "roomJoin": true,
                    "canPublish": publish,
                    "canSubscribe": true,
                    "canPublishData": publish,
                })
            );
        }
    }

    #[test]
    fn empty_display_name_omits_name_claim() {
        let t = decide(Some(&cfg()), "camp", "u", "", Role::Gm, NOW)
            .unwrap()
            .token;
        assert!(decode(parts(&t)[1]).get("name").is_none());
    }

    #[test]
    fn signature_verifies_only_with_the_same_secret() {
        let t = mint(Role::Gm, "u1");
        assert!(verifies(&t, SECRET));
        assert!(!verifies(&t, "some-other-secret"));
    }

    #[test]
    fn different_users_and_rooms_give_different_tokens() {
        let a = mint(Role::Player, "alice");
        let b = mint(Role::Player, "bob");
        assert_ne!(a, b);
        let c = decide(Some(&cfg()), "other", "alice", "Name", Role::Player, NOW)
            .unwrap()
            .token;
        assert_ne!(a, c);
    }

    #[test]
    fn decision_fills_the_contract_message() {
        let m = decide(Some(&cfg()), "camp", "u", "N", Role::Spectator, NOW).unwrap();
        assert_eq!(m.sfu_url, "wss://sfu.example.test");
        assert_eq!(m.room, "tt-camp");
        assert!(!m.can_publish);
        assert_eq!(m.expires_at_ms, 1_700_000_600_000);
    }

    #[test]
    fn unspecified_role_gets_no_token() {
        assert_eq!(
            decide(Some(&cfg()), "camp", "u", "N", Role::Unspecified, NOW),
            Err(MediaDenied::NotPermitted)
        );
    }

    #[test]
    fn not_configured_cases() {
        assert_eq!(
            decide(None, "camp", "u", "N", Role::Gm, NOW),
            Err(MediaDenied::NotConfigured)
        );
        assert_eq!(
            decide(Some(&cfg()), "", "u", "N", Role::Gm, NOW),
            Err(MediaDenied::NotConfigured)
        );
        assert_eq!(room_name(""), None);
        assert_eq!(room_name("x").as_deref(), Some("tt-x"));
        assert_eq!(MediaDenied::NotConfigured.code(), "media_not_configured");
        assert_eq!(MediaDenied::NotPermitted.code(), "not_permitted");
    }

    #[test]
    fn extreme_clock_values_do_not_panic() {
        for now in [i64::MIN, i64::MAX, -1, -1000, 0] {
            let m = decide(Some(&cfg()), "camp", "u", "N", Role::Gm, now).unwrap();
            assert_eq!(parts(&m.token).len(), 3);
        }
        // Negative time floors, it does not truncate toward zero.
        let t = mint_token(&cfg(), "r", "u", "", true, -1, 600).unwrap();
        assert_eq!(decode(parts(&t)[1])["nbf"], -1);
    }

    #[test]
    fn debug_hides_the_secret() {
        let shown = format!("{:?}", cfg());
        assert!(!shown.contains(SECRET));
        assert!(shown.contains("<redacted>"));
        assert!(shown.contains("wss://sfu.example.test"));
    }

    #[test]
    fn config_validation() {
        assert!(MediaConfig::new("wss://a.test", "k", "s").is_ok());
        assert!(MediaConfig::new("ws://localhost:7880", "k", "s").is_ok());
        for bad in ["", "https://a.test", "a.test", "wss://", "ws://"] {
            assert_eq!(
                MediaConfig::new(bad, "k", "s"),
                Err(MediaConfigError::BadUrl),
                "{bad:?}"
            );
        }
        assert_eq!(
            MediaConfig::new("wss://a.test", "", "s"),
            Err(MediaConfigError::EmptyApiKey)
        );
        assert_eq!(
            MediaConfig::new("wss://a.test", "k", ""),
            Err(MediaConfigError::EmptyApiSecret)
        );
        // Errors never carry the secret.
        let e = MediaConfig::new("bad", "k", SECRET).unwrap_err();
        assert!(!format!("{e} {e:?}").contains(SECRET));
    }
}
