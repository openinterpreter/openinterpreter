//! Reads the root authorization version selected by the extension context policy.

use codex_guardian_reviewer::ReviewContextPolicy;

use crate::codex_thread::GuardianAuthorizationVersion;
use crate::session::session::Session;

pub(super) async fn root_review_version(
    policy: ReviewContextPolicy,
    session: &Session,
) -> Option<(GuardianAuthorizationVersion, u64)> {
    if matches!(
        policy,
        ReviewContextPolicy::Legacy | ReviewContextPolicy::LegacyWithCheckpointReuse
    ) {
        return None;
    }

    pub(super) async fn root_authorization_version(
        self,
        session: &Session,
    ) -> Option<GuardianAuthorizationVersion> {
        if self != Self::ThreadOwned {
            return None;
        }
        session
            .services
            .agent_control
            .root_user_authorization(session.thread_id)
            .await
            .map(|snapshot| snapshot.authorization_version)
    }

    pub(super) fn parent_compaction(
        self,
        history: &ContextManager,
        reviewer_compaction_hash: Option<&str>,
    ) -> anyhow::Result<Option<ResponseItem>> {
        let strict = self == Self::ThreadOwned;
        if self == Self::Legacy {
            return Ok(None);
        }
        let Some(checkpoint) =
            codex_history::CompactionCheckpoint::latest(history.annotated_items())
        else {
            return Ok(None);
        };
        let valid = checkpoint.is_usable();
        if !valid && !strict {
            return Ok(None);
        }
        anyhow::ensure!(
            valid,
            "parent compaction checkpoint is unusable for Guardian review"
        );
        if strict {
            // A resumed parent may now use a different model. Compare the actual
            // checkpoint producer with the selected reviewer, not the live parent model.
            anyhow::ensure!(
                checkpoint.is_compatible_with(reviewer_compaction_hash),
                "parent compaction checkpoint is incompatible with the Guardian review model or its compatibility is unknown"
            );
        }
        Ok(Some(checkpoint.item.clone()))
    }
}
