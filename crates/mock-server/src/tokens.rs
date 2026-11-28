//! Token intents: hit points, conditions, create and remove. Plus the small
//! hit point helpers `use_ability` shares.

use tt_contracts::v1::{
    event, Creature, CreatureSize, CreatureUpserted, HealthState, Role, SetConditionsIntent,
    SetHitPointsIntent, Token, TokenRemoved, TokenResource, TokenUpserted,
};

use crate::{Before, Game, Group, Rejection, UndoRecord};

/// Most conditions one token may carry.
const MAX_CONDITIONS: usize = 32;
/// Most tokens in the campaign.
const MAX_TOKENS: usize = 500;
/// Longest token name, in characters.
const MAX_NAME_CHARS: usize = 120;
/// Largest temporary hit point value.
const MAX_TEMP_HP: i32 = 999;

/// Give `token` its live resource pools from `creature` (the template). Only
/// the initial fill: the mock never re-imports a sheet, so there is nothing to
/// reconcile. Pools start at the sheet's `current` clamped to `0..=max`; the
/// spent recharge list starts empty.
pub(crate) fn fill_live_state(token: &mut Token, creature: &Creature) {
    token.resources = creature
        .resources
        .iter()
        .map(|r| TokenResource {
            key: r.key.clone(),
            label: r.label.clone(),
            current: r.current.clamp(0, r.max.max(0)),
            max: r.max.max(0),
        })
        .collect();
    token.recharge_spent_ability_ids.clear();
}

/// Does this token have hit points to change?
pub(crate) fn has_hp(token: &Token) -> bool {
    token.hp_max.is_some_and(|m| m > 0)
}

/// The health state for these hit points (thresholds in scene.proto).
fn health_for(current: i32, max: i32) -> HealthState {
    if current <= 0 {
        HealthState::Down
    } else if i64::from(current) * 4 <= i64::from(max) {
        HealthState::NearDeath
    } else if i64::from(current) * 2 <= i64::from(max) {
        HealthState::Bloodied
    } else {
        HealthState::Healthy
    }
}

/// Recompute `health_state`; a token with no hp_max keeps the one it has.
pub(crate) fn refresh_health(token: &mut Token) {
    if let (Some(current), Some(max)) = (token.hp_current, token.hp_max) {
        if max > 0 {
            token.health_state = health_for(current, max) as i32;
        }
    }
}

/// Take `amount` (not negative) off temporary HP first, then current HP, which
/// floors at 0. Returns true if this took the token from above 0 to 0.
pub(crate) fn apply_damage(token: &mut Token, amount: i32) -> bool {
    let max = token.hp_max.unwrap_or(0);
    let before = token.hp_current.unwrap_or(max);
    let temp = token.hp_temp.unwrap_or(0);
    let absorbed = temp.min(amount);
    if absorbed > 0 {
        token.hp_temp = Some(temp - absorbed);
    }
    let after = (before - (amount - absorbed)).max(0);
    token.hp_current = Some(after);
    refresh_health(token);
    before > 0 && after == 0
}

/// Heal up to `hp_max`. Returns the hit points actually restored.
pub(crate) fn apply_healing(token: &mut Token, amount: i32) -> i32 {
    let max = token.hp_max.unwrap_or(0);
    let before = token.hp_current.unwrap_or(max);
    let after = (before + amount.max(0)).min(max).max(before);
    token.hp_current = Some(after);
    refresh_health(token);
    after - before
}

/// `[a-z0-9_]{1,40}`.
fn valid_condition_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Game {
    pub(crate) fn set_hit_points(
        &mut self,
        user_id: &str,
        s: &SetHitPointsIntent,
    ) -> Result<Group, Rejection> {
        let index = self.controlled_token(user_id, &s.token_id)?;
        let token = &mut self.tokens[index];
        let Some(max) = token.hp_max.filter(|m| *m > 0) else {
            return Err((
                "no_hit_points",
                format!("token {:?} has no hit points", s.token_id),
            ));
        };
        if s.hp_current < 0 || s.hp_current > max || s.hp_temp < 0 || s.hp_temp > MAX_TEMP_HP {
            return Err((
                "bad_hit_points",
                format!(
                    "hp_current must be 0 to {max} and hp_temp 0 to {MAX_TEMP_HP}, got {} and {}",
                    s.hp_current, s.hp_temp
                ),
            ));
        }
        let before = token.clone();
        token.hp_current = Some(s.hp_current);
        token.hp_temp = Some(s.hp_temp);
        refresh_health(token);
        Ok(token_group(index, before, token.clone()))
    }

    pub(crate) fn set_conditions(
        &mut self,
        user_id: &str,
        c: &SetConditionsIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may set conditions")?;
        // Lengths first, as in the real server.
        if c.add.len() + c.remove.len() > 2 * MAX_CONDITIONS {
            return Err((
                "too_many_conditions",
                format!(
                    "add and remove together may list at most {} ids",
                    2 * MAX_CONDITIONS
                ),
            ));
        }
        let Some(index) = self.tokens.iter().position(|t| t.token_id == c.token_id) else {
            return Err(("unknown_token", format!("no token {:?}", c.token_id)));
        };
        if let Some(bad) = c
            .add
            .iter()
            .chain(&c.remove)
            .find(|id| !valid_condition_id(id))
        {
            return Err((
                "bad_condition_id",
                format!("{bad:?} is not [a-z0-9_]{{1,40}}"),
            ));
        }
        if let Some(both) = c.add.iter().find(|id| c.remove.contains(id)) {
            return Err((
                "conflicting_conditions",
                format!("{both:?} is in both add and remove"),
            ));
        }
        let token = &mut self.tokens[index];
        let before = token.clone();
        let mut conditions = token.conditions.clone();
        conditions.retain(|id| !c.remove.contains(id));
        for id in &c.add {
            if !conditions.contains(id) {
                conditions.push(id.clone());
            }
        }
        if conditions.len() > MAX_CONDITIONS {
            return Err((
                "too_many_conditions",
                format!("a token may carry at most {MAX_CONDITIONS} conditions"),
            ));
        }
        token.conditions = conditions;
        Ok(token_group(index, before, token.clone()))
    }

    pub(crate) fn create_token(
        &mut self,
        user_id: &str,
        c: &tt_contracts::v1::CreateTokenIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may create tokens")?;
        let Some(scene) = self
            .template
            .scenes
            .iter()
            .find(|s| s.scene_id == c.scene_id)
        else {
            return Err(("unknown_scene", format!("no scene {:?}", c.scene_id)));
        };
        let level_id = if c.level_id.is_empty() {
            scene
                .levels
                .first()
                .map(|l| l.level_id.clone())
                .unwrap_or_default()
        } else if scene.levels.iter().any(|l| l.level_id == c.level_id) {
            c.level_id.clone()
        } else {
            return Err(("unknown_level", format!("no level {:?}", c.level_id)));
        };
        let creature: Option<&Creature> = if c.creature_id.is_empty() {
            None
        } else {
            match self
                .template
                .creatures
                .iter()
                .find(|cr| cr.creature_id == c.creature_id)
            {
                Some(cr) => Some(cr),
                None => {
                    return Err((
                        "unknown_creature",
                        format!("no creature {:?}", c.creature_id),
                    ))
                }
            }
        };
        let position = match c.position {
            Some(p) if p.x.is_finite() && p.y.is_finite() => p,
            _ => {
                return Err((
                    "bad_coordinate",
                    "position must be set and finite".to_string(),
                ))
            }
        };
        if c.name.chars().count() > MAX_NAME_CHARS {
            return Err((
                "name_too_long",
                format!("a token name may have at most {MAX_NAME_CHARS} characters"),
            ));
        }
        for user in &c.controller_user_ids {
            let Some(member) = self.template.members.iter().find(|m| &m.user_id == user) else {
                return Err(("unknown_user", format!("no member {user:?}")));
            };
            if member.role != Role::Player as i32 {
                return Err((
                    "invalid_controller",
                    format!("{user:?} is not a player and cannot control a token"),
                ));
            }
        }
        if self.tokens.len() >= MAX_TOKENS {
            return Err((
                "too_many_tokens",
                format!("the campaign already has {MAX_TOKENS} tokens"),
            ));
        }

        let name = if !c.name.is_empty() {
            c.name.clone()
        } else if let Some(cr) = creature {
            cr.name.clone()
        } else {
            "Token".to_string()
        };
        let size = CreatureSize::try_from(c.size)
            .ok()
            .filter(|s| *s != CreatureSize::Unspecified)
            .or_else(|| {
                creature
                    .and_then(|cr| CreatureSize::try_from(cr.size).ok())
                    .filter(|s| *s != CreatureSize::Unspecified)
            })
            .unwrap_or(CreatureSize::Medium);
        self.next_token += 1;
        let mut token = Token {
            token_id: format!("token-new-{}", self.next_token),
            scene_id: c.scene_id.clone(),
            level_id,
            creature_id: c.creature_id.clone(),
            name,
            position: Some(position),
            size: size as i32,
            health_state: HealthState::Healthy as i32,
            controller_user_ids: c.controller_user_ids.clone(),
            hidden: c.hidden,
            ..Default::default()
        };
        if let Some(cr) = creature {
            if cr.hp_max > 0 {
                token.hp_current = Some(cr.hp_current);
                token.hp_max = Some(cr.hp_max);
                token.hp_temp = Some(cr.hp_temp);
                refresh_health(&mut token);
            }
            fill_live_state(&mut token, cr);
        }
        let creature_event = creature
            .filter(|_| !c.controller_user_ids.is_empty())
            .cloned();
        let token_id = token.token_id.clone();
        let index = self.tokens.len();
        self.tokens.push(token.clone());

        let mut events = vec![(
            event::Kind::TokenUpserted(TokenUpserted { token: Some(token) }),
            Vec::new(),
        )];
        if let Some(creature) = creature_event {
            events.push((
                event::Kind::CreatureUpserted(CreatureUpserted {
                    creature: Some(creature),
                }),
                Vec::new(),
            ));
        }
        Ok(Group {
            events,
            undo: Some(UndoRecord {
                moved: false,
                before: vec![Before {
                    index,
                    token_id,
                    token: None,
                }],
            }),
        })
    }

    pub(crate) fn remove_token(
        &mut self,
        user_id: &str,
        r: &tt_contracts::v1::RemoveTokenIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may remove tokens")?;
        let Some(index) = self.tokens.iter().position(|t| t.token_id == r.token_id) else {
            return Err(("unknown_token", format!("no token {:?}", r.token_id)));
        };
        if self
            .encounters
            .iter()
            .any(|e| e.combatants.iter().any(|c| c.token_id == r.token_id))
        {
            return Err((
                "in_encounter",
                "the token is in a running encounter; end the encounter first".to_string(),
            ));
        }
        let before = self.tokens.remove(index);
        let mut group = Group::one(event::Kind::TokenRemoved(TokenRemoved {
            token_id: r.token_id.clone(),
        }));
        group.undo = Some(UndoRecord {
            moved: false,
            before: vec![Before {
                index,
                token_id: r.token_id.clone(),
                token: Some(before),
            }],
        });
        Ok(group)
    }
}

/// One `TokenUpserted` for a token that changed, undoable back to `before`.
fn token_group(index: usize, before: Token, after: Token) -> Group {
    let token_id = after.token_id.clone();
    let mut group = Group::one(event::Kind::TokenUpserted(TokenUpserted {
        token: Some(after),
    }));
    group.undo = Some(UndoRecord {
        moved: false,
        before: vec![Before {
            index,
            token_id,
            token: Some(before),
        }],
    });
    group
}
