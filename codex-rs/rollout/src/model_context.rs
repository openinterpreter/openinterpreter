//! Collects persisted rollout items needed to reconstruct the most recent context window.
//!
//! Storage readers feed items newest-to-oldest. The scan stops at the newest compaction that has
//! both replacement history and a window number and a compatible completed turn context with a
//! durable user or full-world-state baseline. Missing fields and rollback require full replay.
//! Items are returned in chronological order; callers prepend canonical head metadata.

use crate::ResponseItemEnvelope;
use crate::RolloutItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InterAgentCommunication;

/// Whether a reverse model-context scan needs more rollout items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelContextScanProgress {
    /// The reader should provide the next older rollout item.
    Continue,
    /// The scan has collected a safe bounded suffix.
    Complete,
}

/// Finds a bounded suffix for reconstructing the most recent context window.
///
/// A compaction with replacement history and a window number is a conversation-history boundary
/// only after a compatible completed turn establishes its context baseline. A retained full world
/// state after that turn's compaction can serve as its baseline even for an empty-input wake.
/// Legacy rollback or an incomplete compaction require the entire rollout for reconstruction.
#[derive(Debug, Default)]
pub struct ModelContextScan {
    items_newest_first: Vec<RolloutItem>,
    saw_compaction: bool,
    saw_completed_turn_context: bool,
    must_scan_to_start: bool,
    active_segment: ActiveTurnSegment,
}

impl ModelContextScan {
    /// Adds the next newest-to-oldest rollout item and reports whether the reader can stop.
    pub fn push(&mut self, item: RolloutItem) -> ModelContextScanProgress {
        let progress = self.observe(&item);
        self.items_newest_first.push(item);
        progress
    }

    /// Returns the collected items in chronological order.
    ///
    /// Call this after the reader reaches the beginning of its source or after [`Self::push`]
    /// returns [`ModelContextScanProgress::Complete`].
    pub fn finish(mut self) -> Vec<RolloutItem> {
        self.items_newest_first.reverse();
        self.items_newest_first
    }

    fn observe(&mut self, item: &RolloutItem) -> ModelContextScanProgress {
        if self.must_scan_to_start {
            return ModelContextScanProgress::Continue;
        }

        match item {
            RolloutItem::Compacted(compacted)
                if compacted.replacement_history.is_none() || compacted.window_number.is_none() =>
            {
                self.must_scan_to_start = true;
            }
            RolloutItem::Compacted(_) => {
                self.saw_compaction = true;
                self.active_segment.saw_compaction = true;
            }
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(_)) => {
                // Rollback markers only occur in legacy history. Keep old rollouts correct rather than
                // duplicating rollback survival semantics in this bounded selector.
                self.must_scan_to_start = true;
            }
            RolloutItem::EventMsg(EventMsg::ItemCompleted(event)) => {
                if self.active_segment.turn_id.is_none() {
                    self.active_segment.turn_id = Some(event.turn_id.clone());
                }
                if turn_ids_are_compatible(
                    self.active_segment.turn_id.as_deref(),
                    Some(event.turn_id.as_str()),
                ) {
                    self.active_segment.has_user_turn |=
                        matches!(&event.item, TurnItem::UserMessage(_));
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnComplete(event)) => {
                self.active_segment
                    .turn_id
                    .get_or_insert_with(|| event.turn_id.clone());
            }
            RolloutItem::EventMsg(EventMsg::TurnAborted(event)) => {
                if let Some(turn_id) = &event.turn_id {
                    self.active_segment
                        .turn_id
                        .get_or_insert_with(|| turn_id.clone());
                }
            }
            RolloutItem::EventMsg(EventMsg::TurnStarted(event)) => {
                if turn_ids_are_compatible(
                    self.active_segment.turn_id.as_deref(),
                    Some(event.turn_id.as_str()),
                ) {
                    self.finalize_active_segment();
                }
            }
            RolloutItem::TurnContext(context) => {
                if self.active_segment.turn_id.is_none() {
                    self.active_segment.turn_id = context.turn_id.clone();
                }
                if turn_ids_are_compatible(
                    self.active_segment.turn_id.as_deref(),
                    context.turn_id.as_deref(),
                ) {
                    self.active_segment.has_turn_context = true;
                }
            }
            RolloutItem::ResponseItem(response_item) => {
                self.active_segment.has_user_turn |=
                    response_item_counts_as_user_turn(response_item);
            }
            RolloutItem::InterAgentCommunication(_) => {
                self.active_segment.has_user_turn = true;
            }
            RolloutItem::EventMsg(EventMsg::UserMessage(_)) => {
                self.active_segment.has_user_turn = true;
            }
            RolloutItem::WorldState(state) => {
                self.active_segment.has_full_world_state |=
                    state.full && !self.active_segment.saw_compaction;
            }
            RolloutItem::EventMsg(_)
            | RolloutItem::DynamicToolsUpdated { .. }
            | RolloutItem::SessionMeta(_)
            | RolloutItem::InterAgentCommunicationMetadata { .. }
            | RolloutItem::RealtimeItem(_)
            | RolloutItem::RetainedContext(_)
            | RolloutItem::SecurityRiskScore(_)
            | RolloutItem::TokenUsageRecord(_) => {}
        }

        if self.has_bounded_cutoff() {
            ModelContextScanProgress::Complete
        } else {
            ModelContextScanProgress::Continue
        }
    }

    fn finalize_active_segment(&mut self) {
        if self.active_segment.has_turn_context
            && (self.active_segment.has_user_turn || self.active_segment.has_full_world_state)
        {
            self.saw_completed_turn_context = true;
        }
        self.active_segment = ActiveTurnSegment::default();
    }

    fn has_bounded_cutoff(&self) -> bool {
        !self.must_scan_to_start && self.saw_compaction && self.saw_completed_turn_context
    }
}

#[derive(Debug, Default)]
struct ActiveTurnSegment {
    turn_id: Option<String>,
    has_user_turn: bool,
    has_turn_context: bool,
    has_full_world_state: bool,
    saw_compaction: bool,
}

fn turn_ids_are_compatible(active_turn_id: Option<&str>, item_turn_id: Option<&str>) -> bool {
    active_turn_id
        .is_none_or(|turn_id| item_turn_id.is_none_or(|item_turn_id| item_turn_id == turn_id))
}

fn response_item_counts_as_user_turn(response_item: &ResponseItemEnvelope) -> bool {
    match &response_item.item {
        ResponseItem::AgentMessage { .. } => true,
        ResponseItem::Message { role, content, .. } => {
            role == "assistant" && InterAgentCommunication::is_message_content(content)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::ThreadId;

    #[test]
    fn dynamic_tool_catalog_is_retained_without_creating_a_context_boundary() {
        let mut scan = ModelContextScan::default();
        assert_eq!(
            scan.push(RolloutItem::DynamicToolsUpdated {
                thread_id: ThreadId::new(),
                tools: vec![],
            }),
            ModelContextScanProgress::Continue
        );
        assert!(matches!(
            scan.finish().as_slice(),
            [RolloutItem::DynamicToolsUpdated { tools, .. }] if tools.is_empty()
        ));
    }
}
