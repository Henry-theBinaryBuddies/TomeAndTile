//! `UndoIntent`: reverse the latest undoable group by restoring the tokens it
//! changed. The mock keeps no dependency check; see the README.

use tt_contracts::v1::{
    event, EventUndone, Token, TokenMoved, TokenRemoved, TokenUpserted, UndoIntent,
};

use crate::{not_permitted, Game, Group, Rejection};

/// One token's state before a group changed it.
pub(crate) struct Before {
    /// Where the token sat in the token list, so a removed one goes back there.
    pub index: usize,
    pub token_id: String,
    /// `None`: the group created the token.
    pub token: Option<Token>,
}

/// What undoing an undoable group restores.
pub(crate) struct UndoRecord {
    /// A move is undone with `TokenMoved`, everything else with an upsert.
    pub moved: bool,
    pub before: Vec<Before>,
}

pub(crate) struct UndoGroup {
    pub first_seq: u64,
    pub record: UndoRecord,
}

/// The seq range of one intent's events.
pub(crate) struct GroupRange {
    pub first: u64,
    pub last: u64,
    pub undoable: bool,
    pub undone: bool,
}

impl Game {
    pub(crate) fn undo(&mut self, user_id: &str, u: &UndoIntent) -> Result<Group, Rejection> {
        self.gm_only(user_id, "only the GM may undo")?;
        let seq = u.event_seq;
        let pos = self.groups.partition_point(|g| g.last < seq);
        let range = self
            .groups
            .get(pos)
            .filter(|g| g.first <= seq)
            .ok_or(("unknown_event", format!("no event {seq}")))?;
        if seq != range.first {
            return Err((
                "not_first_event",
                format!(
                    "event {seq} is not the first of its group; name {}",
                    range.first
                ),
            ));
        }
        if !range.undoable {
            return Err(("not_undoable", format!("event {seq} cannot be undone")));
        }
        if range.undone {
            return Err((
                "already_undone",
                format!("the group at {seq} was already undone"),
            ));
        }
        let (first, last) = (range.first, range.last);
        if self.undo_stack.last().map(|g| g.first_seq) != Some(first) {
            return Err((
                "not_supported_by_mock",
                "the mock undoes only the most recent group that has not been undone".to_string(),
            ));
        }
        let Some(group) = self.undo_stack.pop() else {
            return Err(not_permitted("nothing to undo"));
        };
        self.groups[pos].undone = true;

        let mut events = Vec::new();
        for before in group.record.before {
            let exists = self
                .tokens
                .iter()
                .position(|t| t.token_id == before.token_id);
            let kind = match before.token {
                Some(token) => {
                    match exists {
                        Some(i) => self.tokens[i] = token.clone(),
                        None => {
                            let at = before.index.min(self.tokens.len());
                            self.tokens.insert(at, token.clone());
                        }
                    }
                    match (group.record.moved, token.position) {
                        (true, Some(position)) => event::Kind::TokenMoved(TokenMoved {
                            token_id: token.token_id,
                            path: vec![position],
                            level_id: token.level_id,
                            distance_feet: 0.0,
                        }),
                        _ => event::Kind::TokenUpserted(TokenUpserted { token: Some(token) }),
                    }
                }
                None => {
                    if let Some(i) = exists {
                        self.tokens.remove(i);
                    }
                    event::Kind::TokenRemoved(TokenRemoved {
                        token_id: before.token_id,
                    })
                }
            };
            events.push((kind, Vec::new()));
        }
        for undone_seq in first..=last {
            events.push((
                event::Kind::EventUndone(EventUndone { undone_seq }),
                Vec::new(),
            ));
        }
        Ok(Group { events, undo: None })
    }
}
