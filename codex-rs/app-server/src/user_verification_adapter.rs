//! Validates local verification requests and maps native results into the app-server API.
//! Native diagnostics stay private; clients receive bounded input errors and typed reasons.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use codex_app_server_protocol as rpc;
use codex_user_verification as native;

pub(super) fn validate(
    params: rpc::UserVerificationVerifyParams,
) -> Result<native::UserVerificationRequest, rpc::JSONRPCErrorError> {
    let invalid = || {
        error(
            rpc::UserVerificationErrorDetails::InvalidRequest {
                reason: rpc::UserVerificationInvalidRequestReason::InvalidParams,
            },
            "Invalid verification challenge or display text.",
        )
    };
    if params.challenge.len() > 5462
        || params.title.is_empty()
        || params.title.len() > 256
        || params.description.len() > 4096
    {
        return Err(invalid());
    }
    let challenge = URL_SAFE_NO_PAD
        .decode(&params.challenge)
        .map_err(|_| invalid())?;
    if challenge.is_empty() || challenge.len() > 4096 {
        return Err(invalid());
    }
    Ok(native::UserVerificationRequest {
        challenge,
        title: params.title,
        description: params.description,
    })
}

pub(crate) fn unavailable() -> rpc::JSONRPCErrorError {
    error(
        rpc::UserVerificationErrorDetails::Unavailable {
            reason: rpc::UserVerificationUnavailableReason::ProviderUnavailable,
        },
        "User verification is not available in this build or account.",
    )
}

pub(super) fn unavailable_reason(
    reason: native::UserVerificationUnavailableReason,
) -> rpc::UserVerificationUnavailableReason {
    match reason {
        native::UserVerificationUnavailableReason::CredentialMissing => {
            rpc::UserVerificationUnavailableReason::CredentialMissing
        }
        native::UserVerificationUnavailableReason::BiometricsUnavailable => {
            rpc::UserVerificationUnavailableReason::BiometricsUnavailable
        }
        native::UserVerificationUnavailableReason::ProviderUnavailable => {
            rpc::UserVerificationUnavailableReason::ProviderUnavailable
        }
    }
}

pub(super) fn native_error(value: native::UserVerificationError) -> rpc::JSONRPCErrorError {
    let (details, message) = match value {
        native::UserVerificationError::Unavailable { reason, .. } => (
            rpc::UserVerificationErrorDetails::Unavailable {
                reason: unavailable_reason(reason),
            },
            "Local user verification is unavailable.",
        ),
        native::UserVerificationError::Cancelled { reason, .. } => (
            rpc::UserVerificationErrorDetails::Cancelled {
                reason: match reason {
                    native::UserVerificationCancellationReason::UserCancelled => {
                        rpc::UserVerificationCancellationReason::UserCancelled
                    }
                    native::UserVerificationCancellationReason::Interrupted => {
                        rpc::UserVerificationCancellationReason::Interrupted
                    }
                },
            },
            "User verification was cancelled.",
        ),
        native::UserVerificationError::Failed { reason, .. } => (
            rpc::UserVerificationErrorDetails::Failed {
                reason: match reason {
                    native::UserVerificationFailureReason::AuthenticationFailed => {
                        rpc::UserVerificationFailureReason::AuthenticationFailed
                    }
                    native::UserVerificationFailureReason::Timeout => {
                        rpc::UserVerificationFailureReason::Timeout
                    }
                    native::UserVerificationFailureReason::ProviderError => {
                        rpc::UserVerificationFailureReason::ProviderError
                    }
                },
            },
            "User verification failed.",
        ),
    };
    error(details, message)
}

pub(super) fn error(
    data: rpc::UserVerificationErrorDetails,
    message: &str,
) -> rpc::JSONRPCErrorError {
    rpc::JSONRPCErrorError {
        code: if matches!(
            data,
            rpc::UserVerificationErrorDetails::InvalidRequest { .. }
        ) {
            -32602
        } else {
            -32603
        },
        message: message.into(),
        data: Some(
            serde_json::to_value(data).unwrap_or_else(
                |_| serde_json::json!({"type": "failed", "reason": "providerError"}),
            ),
        ),
    }
}

#[cfg(test)]
#[path = "user_verification_adapter_tests.rs"]
mod tests;
