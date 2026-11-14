//! Tracks admitted transcript progress separately from completed reviews and committed history.
//! Hosts serialize reviews and checkpoint commits, and supply opaque completed history.
//! An exclusive continuation can take the history without advancing live progress.

use codex_guardian_context::TranscriptCursor;

/// A completed history and the transcript progress that produced it.
#[derive(Clone)]
pub struct ConversationCheckpoint<H> {
    history: H,
    cursor: Option<TranscriptCursor>,
    completed_review_count: usize,
}

impl<H> ConversationCheckpoint<H> {
    pub fn history(&self) -> &H {
        &self.history
    }
}

/// Shared bookkeeping for a host-owned Guardian conversation.
pub struct ConversationState<H> {
    cursor: Option<TranscriptCursor>,
    completed_review_count: usize,
    checkpoint: Option<ConversationCheckpoint<H>>,
}

impl<H> Default for ConversationState<H> {
    fn default() -> Self {
        Self {
            cursor: None,
            completed_review_count: 0,
            checkpoint: None,
        }
    }
}

impl<H> From<ConversationCheckpoint<H>> for ConversationState<H> {
    /// Restores committed progress and keeps its history available for rollback or a fork.
    /// An exclusive request can transfer the history with `take_history` after admission.
    fn from(checkpoint: ConversationCheckpoint<H>) -> Self {
        Self {
            cursor: checkpoint.cursor,
            completed_review_count: checkpoint.completed_review_count,
            checkpoint: Some(checkpoint),
        }
    }
}

impl<H> ConversationState<H> {
    pub fn cursor(&self) -> Option<TranscriptCursor> {
        self.cursor
    }

    pub fn completed_review_count(&self) -> usize {
        self.completed_review_count
    }

    /// Rebuilds the active transcript after compaction. Forks can still use the last
    /// committed history; it owns its own cursor and is independent of this reset.
    pub fn reset_transcript(&mut self) {
        self.cursor = None;
    }

    /// Records input retained by the live conversation, even if inference failed.
    /// This does not complete a review or change the committed fork checkpoint.
    pub fn admit_transcript(&mut self, cursor: TranscriptCursor) {
        self.cursor = Some(cursor);
    }

    /// Advances live progress once the host's review completes.
    pub fn complete_review(&mut self, cursor: TranscriptCursor) {
        self.admit_transcript(cursor);
        self.completed_review_count = self.completed_review_count.saturating_add(1);
    }

    /// Pairs host-supplied history with the current progress for subsequent forks.
    pub fn commit_snapshot(&mut self, history: H) {
        self.checkpoint = Some(ConversationCheckpoint {
            history,
            cursor: self.cursor,
            completed_review_count: self.completed_review_count,
        });
    }

    pub fn snapshot(&self) -> Option<&ConversationCheckpoint<H>> {
        self.checkpoint.as_ref()
    }

    /// Transfers committed history to its exclusive owner, leaving progress unchanged.
    /// The host must commit a completed replacement before this state can be reused.
    pub fn take_history(&mut self) -> Option<H> {
        self.checkpoint.take().map(|checkpoint| checkpoint.history)
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
