//! Identity for version 0.
//!
//! DEV AUTH IS NOT SECURITY. There are no accounts yet (ADR 0008). With
//! `TT_DEV_AUTH=1` anyone who can reach the socket can claim to be any member
//! of the seeded campaign by sending `dev:<user_id>`. Never enable it on a
//! server reachable from the internet. With it off, every Hello is refused.

use tt_contracts::v1::Hello;

use crate::campaign::Campaign;
use crate::view::Viewer;

/// Why a Hello was refused. The socket is closed; no detail is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    /// No login method is enabled.
    Disabled,
    /// Token missing, malformed or not a `dev:` token.
    BadToken,
    /// The named user is not a member of the campaign.
    UnknownMember,
    /// The Hello names a different campaign.
    WrongCampaign,
}

/// Pull `<user_id>` out of `dev:<user_id>`. Empty ids are refused.
pub fn parse_dev_token(token: &str) -> Option<&str> {
    token.strip_prefix("dev:").filter(|id| !id.is_empty())
}

/// Decide who a Hello is. Never treats a missing or unknown token as a
/// player.
pub fn authenticate(
    campaign: &Campaign,
    dev_auth: bool,
    hello: &Hello,
) -> Result<Viewer, AuthError> {
    if !dev_auth {
        return Err(AuthError::Disabled);
    }
    if !hello.campaign_id.is_empty()
        && !campaign.campaign_id.is_empty()
        && hello.campaign_id != campaign.campaign_id
    {
        return Err(AuthError::WrongCampaign);
    }
    let user_id = parse_dev_token(&hello.session_token).ok_or(AuthError::BadToken)?;
    let role = campaign.role_of(user_id).ok_or(AuthError::UnknownMember)?;
    Ok(Viewer {
        user_id: user_id.to_string(),
        role,
    })
}
