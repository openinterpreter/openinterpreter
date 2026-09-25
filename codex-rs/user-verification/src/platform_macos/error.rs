//! Maps macOS Security and LocalAuthentication failures to stable errors without exposing localized OS diagnostics.

use crate::UserVerificationCancellationReason;
use crate::UserVerificationError;
use crate::UserVerificationFailureReason;
use crate::UserVerificationUnavailableReason;

pub(crate) fn classify(domain: &str, code: i64) -> UserVerificationError {
    tracing::debug!(domain, code, "native user-verification operation failed");
    match (domain, code) {
        ("NSOSStatusErrorDomain", -128) | ("com.apple.LocalAuthentication", -2 | -3) => {
            UserVerificationError::Cancelled {
                reason: UserVerificationCancellationReason::UserCancelled,
                message: "authentication was cancelled".to_string(),
            }
        }
        ("com.apple.LocalAuthentication", -4 | -9) => UserVerificationError::Cancelled {
            reason: UserVerificationCancellationReason::Interrupted,
            message: "authentication was interrupted".to_string(),
        },
        ("NSOSStatusErrorDomain", -25293) | ("com.apple.LocalAuthentication", -1) => {
            UserVerificationError::Failed {
                reason: UserVerificationFailureReason::AuthenticationFailed,
                message: "biometric authentication failed".to_string(),
            }
        }
        ("com.apple.LocalAuthentication", -5 | -6 | -7 | -8 | -12 | -13) => {
            UserVerificationError::Unavailable {
                reason: UserVerificationUnavailableReason::BiometricsUnavailable,
                message: "biometric authentication is not available right now".to_string(),
            }
        }
        ("com.apple.LocalAuthentication", -1004) => UserVerificationError::Unavailable {
            reason: UserVerificationUnavailableReason::ProviderUnavailable,
            message: "authentication UI is not permitted in this process".to_string(),
        },
        ("NSOSStatusErrorDomain", -34018) => UserVerificationError::Unavailable {
            reason: UserVerificationUnavailableReason::ProviderUnavailable,
            message: "this binary is missing the required keychain entitlements".to_string(),
        },
        ("NSOSStatusErrorDomain", -25291 | -25308) => UserVerificationError::Unavailable {
            reason: UserVerificationUnavailableReason::ProviderUnavailable,
            message: "the platform keychain is not available right now".to_string(),
        },
        _ => UserVerificationError::Failed {
            reason: UserVerificationFailureReason::ProviderError,
            message: "the platform could not complete user verification".to_string(),
        },
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
