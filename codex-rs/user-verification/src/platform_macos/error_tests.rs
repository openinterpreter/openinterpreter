//! Native failure classification is based on the domain and code, never localized text.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn cancellation_is_distinct_from_authentication_failure() {
    assert_eq!(
        classify("com.apple.LocalAuthentication", /*code*/ -2),
        UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::UserCancelled,
            message: "authentication was cancelled".to_string(),
        }
    );
    assert_eq!(
        classify("NSOSStatusErrorDomain", /*code*/ -25293),
        UserVerificationError::Failed {
            reason: UserVerificationFailureReason::AuthenticationFailed,
            message: "biometric authentication failed".to_string(),
        }
    );
}

#[test]
fn biometric_lockout_and_missing_entitlements_report_distinct_unavailability() {
    assert_eq!(
        classify("com.apple.LocalAuthentication", /*code*/ -8),
        UserVerificationError::Unavailable {
            reason: UserVerificationUnavailableReason::BiometricsUnavailable,
            message: "biometric authentication is not available right now".to_string(),
        }
    );
    assert_eq!(
        classify("NSOSStatusErrorDomain", /*code*/ -34018),
        UserVerificationError::Unavailable {
            reason: UserVerificationUnavailableReason::ProviderUnavailable,
            message: "this binary is missing the required keychain entitlements".to_string(),
        }
    );
}

#[test]
fn unknown_error_domain_cannot_impersonate_cancellation() {
    assert_eq!(
        classify("unknown", /*code*/ -2),
        UserVerificationError::Failed {
            reason: UserVerificationFailureReason::ProviderError,
            message: "the platform could not complete user verification".to_string(),
        }
    );
}
