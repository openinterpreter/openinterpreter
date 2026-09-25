//! Validates the wire-to-native boundary before any provider work can start.

use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;

#[test]
fn invalid_challenge_and_display_values_are_rejected_before_native_work() {
    for (challenge, title, description) in [
        ("".into(), "Approve".into(), String::new()),
        ("AQ==".into(), "Approve".into(), String::new()),
        (
            URL_SAFE_NO_PAD.encode(vec![0; 4097]),
            "Approve".into(),
            String::new(),
        ),
        ("AQ".into(), String::new(), String::new()),
        ("AQ".into(), "é".repeat(129), String::new()),
        ("AQ".into(), "Approve".into(), "x".repeat(4097)),
    ] {
        let error = validate(rpc::UserVerificationVerifyParams {
            challenge,
            title,
            description,
        })
        .err()
        .unwrap();
        assert_eq!(
            error.data,
            Some(json!({"type": "invalidRequest", "reason": "invalidParams"}))
        );
    }
}
