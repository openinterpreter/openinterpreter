//! Stable error categories, with provider diagnostics kept outside public error messages.

use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserVerificationUnavailableReason {
    CredentialMissing,
    BiometricsUnavailable,
    ProviderUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserVerificationCancellationReason {
    UserCancelled,
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserVerificationFailureReason {
    AuthenticationFailed,
    Timeout,
    ProviderError,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum UserVerificationError {
    #[error("user verification is unavailable: {message}")]
    Unavailable {
        reason: UserVerificationUnavailableReason,
        message: String,
    },
    #[error("user verification was cancelled: {message}")]
    Cancelled {
        reason: UserVerificationCancellationReason,
        message: String,
    },
    #[error("user verification failed: {message}")]
    Failed {
        reason: UserVerificationFailureReason,
        message: String,
    },
}
