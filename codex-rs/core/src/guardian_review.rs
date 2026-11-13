//! Context adapter types used by the host-installed synchronous Guardian extension.
//! The extension supplies agent startup and owns the reviewer pool and its lifecycle.

pub use crate::guardian::GuardianReviewSession;
pub use crate::guardian::GuardianReviewState;
pub use crate::guardian::PreparedGuardianContext;
pub use crate::guardian::prepare_review_prewarm;

/// The isolated assessment of one recorded tool call; no tool is executed or authorized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardianReviewResult {
    pub call_id: String,
    pub assessment: Option<codex_guardian_reviewer::GuardianAssessment>,
}
