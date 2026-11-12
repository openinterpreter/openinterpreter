use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn capture_honors_privacy_and_keeps_oversized_context_action_and_decision() -> anyhow::Result<()> {
    let action = r#"{"command":"git push"}"#;
    for (decision, status) in [
        (
            r#"{"outcome":"deny","rationale":"Missing approval."}"#,
            "denied",
        ),
        (r#"preface {"outcome":"deny"}"#, "denied"),
        ("not valid guardian json", "invalid_decision"),
    ] {
        let thread_id = ThreadId::new();
        let reviewer_thread_id = ThreadId::new();
        let outcome = GuardianReviewSessionOutcome::Completed(Ok(Some(decision.to_owned())));
        for settings in [
            ReviewFeedbackSettings {
                enabled: false,
                ephemeral: false,
            },
            ReviewFeedbackSettings {
                enabled: true,
                ephemeral: true,
            },
        ] {
            assert!(FailedReviewFeedback::for_outcome(&outcome, settings).is_none());
        }
        let feedback = FailedReviewFeedback::for_outcome(
            &outcome,
            ReviewFeedbackSettings {
                enabled: true,
                ephemeral: false,
            },
        )
        .expect("failed review retains feedback");
        feedback.store(ReviewFeedbackContext {
            reviewed_thread_id: thread_id,
            reviewed_turn_id: "parent-turn",
            target_item_id: Some("push-call"),
            reviewer_thread_id,
            model: "review-model",
            action,
            action_truncated: false,
            instructions: Some(&"x".repeat(MAX_RECORD_BYTES)),
            history: Vec::new(),
        });
        let contents = codex_feedback::guardian_review_failures(&[thread_id])
            .attachment
            .expect("failed-review record")
            .buffer;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&contents)?,
            json!({
                "reviewed_thread_id": thread_id,
                "reviewed_turn_id": "parent-turn",
                "target_item_id": "push-call",
                "reviewer_thread_id": reviewer_thread_id,
                "model": "review-model",
                "status": status,
                "decision": decision,
                "action": action,
                "action_truncated": false,
                "instructions": null,
                "history": [],
                "context_omitted": true,
            })
        );
    }
    Ok(())
}
