//! Typed unavailable behavior until a native provider is available for this platform.

use crate::*;

pub(crate) struct UnsupportedProvider;

fn unavailable() -> UserVerificationError {
    UserVerificationError::Unavailable {
        reason: UserVerificationUnavailableReason::ProviderUnavailable,
        message: "this platform does not support user verification".to_string(),
    }
}

impl UserVerificationProvider for UnsupportedProvider {
    fn status(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationStatus, UserVerificationError> {
        guard.check()?;
        Ok(UserVerificationStatus {
            credential: None,
            unavailable_reason: Some(UserVerificationUnavailableReason::ProviderUnavailable),
            unavailable_message: Some(
                "this platform does not support user verification".to_string(),
            ),
        })
    }

    fn ensure_key(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationKeyCreation, UserVerificationError> {
        guard.check()?;
        Err(unavailable())
    }

    fn delete(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationKeyDeletion, UserVerificationError> {
        guard.check()?;
        Err(unavailable())
    }

    fn verify(
        &self,
        _request: &UserVerificationRequest,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationProof, UserVerificationError> {
        guard.check()?;
        Err(unavailable())
    }
}
