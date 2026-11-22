//! Remembers which turn accepted each client message id on a thread.
//!
//! `turn/start` and `turn/steer` may carry a client message id. When a request
//! repeats an id this thread already accepted, the submission is answered with
//! the turn that holds the original input instead of adding the input again.

use std::collections::HashMap;
use std::collections::VecDeque;

use codex_history::RolloutItem;
use codex_protocol::protocol::EventMsg;

/// Number of most recent client message ids remembered per thread.
///
/// Retries follow the original request closely, so a window of recent ids is
/// enough to make them idempotent while keeping memory flat on long threads.
const MAX_ACCEPTED_CLIENT_IDS: usize = 4_096;

/// Insertion-ordered map from client message id to the id of the turn that
/// holds, or will hold, that input.
///
/// The `HashMap` answers the lookup made on every submission, and the
/// `VecDeque` keeps insertion order so the oldest id is evicted first. The
/// bound keeps the linear scan in `remove` cheap, so no ordered-map dependency
/// is needed.
#[derive(Debug, Default)]
pub(super) struct AcceptedClientIds {
    turn_ids: HashMap<String, String>,
    order: VecDeque<String>,
}

impl AcceptedClientIds {
    /// Rebuilds the index from recorded user messages, mapping each id to the
    /// turn whose `TurnStarted` event precedes the message.
    pub(super) fn from_rollout(items: &[RolloutItem]) -> Self {
        let mut accepted = Self::default();
        let mut current_turn_id: Option<&str> = None;
        for item in items {
            let RolloutItem::EventMsg(event) = item else {
                continue;
            };
            match event {
                EventMsg::TurnStarted(event) => current_turn_id = Some(event.turn_id.as_str()),
                EventMsg::UserMessage(event) => {
                    if let Some(client_id) = &event.client_id
                        && let Some(turn_id) = current_turn_id
                    {
                        accepted.insert(client_id.clone(), turn_id.to_string());
                    }
                }
                _ => {}
            }
        }
        accepted
    }

    pub(super) fn turn_id(&self, client_id: &str) -> Option<&str> {
        self.turn_ids.get(client_id).map(String::as_str)
    }

    /// Maps `client_id` to `turn_id`. A known id keeps its position and moves
    /// to the new turn; a new id evicts the oldest one once the bound is hit.
    pub(super) fn insert(&mut self, client_id: String, turn_id: String) {
        if let Some(existing) = self.turn_ids.get_mut(&client_id) {
            *existing = turn_id;
            return;
        }
        self.order.push_back(client_id.clone());
        self.turn_ids.insert(client_id, turn_id);
        if self.order.len() > MAX_ACCEPTED_CLIENT_IDS
            && let Some(oldest) = self.order.pop_front()
        {
            self.turn_ids.remove(&oldest);
        }
    }

    pub(super) fn remove(&mut self, client_id: &str) {
        if self.turn_ids.remove(client_id).is_some() {
            self.order.retain(|id| id.as_str() != client_id);
        }
    }
}

#[cfg(test)]
#[path = "accepted_client_ids_tests.rs"]
mod tests;
