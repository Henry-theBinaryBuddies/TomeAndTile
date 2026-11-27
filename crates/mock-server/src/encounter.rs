//! Encounter intents: start, end turn, set initiative, end. The mock tracks no
//! condition timers, so ending a turn or an encounter never touches a token.

use tt_contracts::v1::{
    event, Combatant, DiceRolled, Encounter, EncounterEnded, EncounterUpdated, EndEncounterIntent,
    EndTurnIntent, RollMode, RollPurpose, SetInitiativeIntent, StartEncounterIntent,
};

use crate::dice::{self, Notation};
use crate::{not_permitted, Game, Group, Rejection, GM_USER};

fn updated(encounter: &Encounter) -> Group {
    Group::one(event::Kind::EncounterUpdated(EncounterUpdated {
        encounter: Some(encounter.clone()),
    }))
}

/// Sort into turn order: higher initiative first, then lower `order`. Stable,
/// so equal keys keep their list order. Renumbers `order` to the new positions.
fn sort_combatants(combatants: &mut [Combatant]) {
    combatants.sort_by(|a, b| b.initiative.cmp(&a.initiative).then(a.order.cmp(&b.order)));
    for (i, c) in combatants.iter_mut().enumerate() {
        c.order = u32::try_from(i).unwrap_or(u32::MAX);
    }
}

impl Game {
    fn encounter_index(&self, encounter_id: &str) -> Result<usize, Rejection> {
        self.encounters
            .iter()
            .position(|e| e.encounter_id == encounter_id)
            .ok_or((
                "unknown_encounter",
                format!("no encounter {encounter_id:?}"),
            ))
    }

    pub(crate) fn start_encounter(
        &mut self,
        user_id: &str,
        s: &StartEncounterIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may start an encounter")?;
        if !self
            .template
            .scenes
            .iter()
            .any(|sc| sc.scene_id == s.scene_id)
        {
            return Err(("unknown_scene", format!("no scene {:?}", s.scene_id)));
        }
        if s.token_ids.is_empty() {
            return Err((
                "no_combatants",
                "an encounter needs at least one token".to_string(),
            ));
        }
        let mut ids: Vec<&String> = Vec::new();
        for id in &s.token_ids {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        for id in &ids {
            if !self
                .tokens
                .iter()
                .any(|t| &t.token_id == *id && t.scene_id == s.scene_id)
            {
                return Err((
                    "unknown_token",
                    format!("token {id:?} does not exist on that scene"),
                ));
            }
        }
        if self.encounters.iter().any(|e| e.scene_id == s.scene_id) {
            return Err((
                "encounter_running",
                "the scene already has a running encounter".to_string(),
            ));
        }

        // Each DiceRolled is the next event, so its seq names its roll.
        let mut events = Vec::new();
        let mut combatants = Vec::new();
        for (i, id) in ids.iter().enumerate() {
            let token = self.tokens.iter().find(|t| &t.token_id == *id);
            let (name, creature_id) = token
                .map(|t| (t.name.clone(), t.creature_id.clone()))
                .unwrap_or_default();
            let bonus = self
                .template
                .creatures
                .iter()
                .find(|c| !creature_id.is_empty() && c.creature_id == creature_id)
                .map_or(0, |c| c.initiative_bonus);
            let seq = self.seq + 1 + u64::try_from(i).unwrap_or(0);
            let mut roll = dice::roll(
                self.rng.as_mut(),
                &Notation::attack(bonus),
                RollMode::Normal,
                format!("roll-{seq}"),
                user_id,
                format!("{name} initiative"),
            )
            .ok_or((
                "not_supported_by_mock",
                "a roll total was out of range".to_string(),
            ))?;
            roll.purpose = RollPurpose::Initiative as i32;
            roll.roller_creature_id = creature_id;
            combatants.push(Combatant {
                token_id: (*id).clone(),
                initiative: roll.total,
                ..Default::default()
            });
            events.push((
                event::Kind::DiceRolled(DiceRolled { roll: Some(roll) }),
                Vec::new(),
            ));
        }
        sort_combatants(&mut combatants);
        self.next_encounter += 1;
        let encounter = Encounter {
            encounter_id: format!("encounter-{}", self.next_encounter),
            scene_id: s.scene_id.clone(),
            round: 1,
            turn_index: 0,
            combatants,
        };
        events.push((
            event::Kind::EncounterUpdated(EncounterUpdated {
                encounter: Some(encounter.clone()),
            }),
            Vec::new(),
        ));
        self.encounters.push(encounter);
        Ok(Group { events, undo: None })
    }

    pub(crate) fn end_turn(
        &mut self,
        user_id: &str,
        e: &EndTurnIntent,
    ) -> Result<Group, Rejection> {
        // The sender's right depends on the encounter, so a missing one is
        // reported first, for the GM and players alike.
        let index = self.encounter_index(&e.encounter_id)?;
        let encounter = &self.encounters[index];
        let current = encounter
            .combatants
            .get(encounter.turn_index as usize)
            .map(|c| c.token_id.as_str())
            .unwrap_or_default();
        let may = user_id == GM_USER
            || self.tokens.iter().any(|t| {
                t.token_id == current && t.controller_user_ids.iter().any(|u| u == user_id)
            });
        if !may {
            return Err(not_permitted(
                "only the GM or the current combatant's controller may end the turn",
            ));
        }
        let encounter = &mut self.encounters[index];
        encounter.turn_index += 1;
        if encounter.turn_index as usize >= encounter.combatants.len() {
            encounter.turn_index = 0;
            encounter.round += 1;
        }
        Ok(updated(encounter))
    }

    pub(crate) fn set_initiative(
        &mut self,
        user_id: &str,
        s: &SetInitiativeIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may set initiative")?;
        let index = self.encounter_index(&s.encounter_id)?;
        let encounter = &mut self.encounters[index];
        let Some(target) = encounter
            .combatants
            .iter()
            .position(|c| c.token_id == s.token_id)
        else {
            return Err((
                "unknown_combatant",
                format!("token {:?} is not in the encounter", s.token_id),
            ));
        };
        let current = encounter
            .combatants
            .get(encounter.turn_index as usize)
            .map(|c| c.token_id.clone())
            .unwrap_or_default();
        encounter.combatants[target].initiative = s.initiative;
        encounter.combatants[target].order = s.order;
        sort_combatants(&mut encounter.combatants);
        if let Some(i) = encounter
            .combatants
            .iter()
            .position(|c| c.token_id == current)
        {
            encounter.turn_index = u32::try_from(i).unwrap_or(0);
        }
        Ok(updated(encounter))
    }

    pub(crate) fn end_encounter(
        &mut self,
        user_id: &str,
        e: &EndEncounterIntent,
    ) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may end an encounter")?;
        let index = self.encounter_index(&e.encounter_id)?;
        let ended = self.encounters.remove(index);
        Ok(Group::one(event::Kind::EncounterEnded(EncounterEnded {
            encounter_id: ended.encounter_id,
        })))
    }
}
