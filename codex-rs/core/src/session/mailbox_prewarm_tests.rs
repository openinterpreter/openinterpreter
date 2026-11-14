//! Checks mailbox turn admission and startup prewarm settings capture.
#![cfg(test)]

use super::*;
use codex_protocol::openai_models::ReasoningEffort;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn prewarm_turn_keeps_its_captured_settings_after_defaults_change() {
    let (session, _turn) = make_session_and_context().await;
    let germany = RequestCountry::from_iso_code("DE").unwrap();
    let united_states = RequestCountry::from_iso_code("US").unwrap();
    session
        .update_settings(SessionSettingsUpdate {
            step_settings: StepSettingsUpdate {
                request_country: Some(germany.clone()),
                effort: Some(Some(ReasoningEffort::Low)),
                ..Default::default()
            },
            ..Default::default()
        })
        .await
        .expect("update thread settings");
    let prewarm = session
        .new_startup_prewarm_turn_with_sub_id(INITIAL_SUBMIT_ID.to_owned())
        .await;
    session
        .update_settings(SessionSettingsUpdate {
            step_settings: StepSettingsUpdate {
                request_country: Some(united_states.clone()),
                effort: Some(Some(ReasoningEffort::High)),
                ..Default::default()
            },
            ..Default::default()
        })
        .await
        .expect("update thread settings");

    assert_eq!(
        (
            prewarm.request_country(),
            prewarm.initial_settings.reasoning_effort()
        ),
        (&germany, Some(&ReasoningEffort::Low))
    );
    let defaults = session.thread_config_snapshot().await;
    assert_eq!(
        (defaults.request_country, defaults.reasoning_effort),
        (united_states, Some(ReasoningEffort::High))
    );
}
