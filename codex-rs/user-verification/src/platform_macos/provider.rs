//! Secure Enclave keys; biometric access is enforced by the key, not a separate UI check.

use super::error;
use super::key_protection;
use crate::UserVerificationError;
use crate::UserVerificationFailureReason;
use crate::UserVerificationKeyCreation;
use crate::UserVerificationKeyDeletion;
use crate::UserVerificationKeyInfo;
use crate::UserVerificationKeyNamespace;
use crate::UserVerificationProof;
use crate::UserVerificationProvider;
use crate::UserVerificationRequest;
use crate::UserVerificationRequestGuard;
use crate::UserVerificationStatus;
use crate::UserVerificationUnavailableReason;
use crate::lifecycle_lock::LifecycleLock;
use crate::native_operation::run_with_cancellation;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use core_foundation::base::CFType;
use core_foundation::base::TCFType as _;
use objc2::rc::Retained;
use objc2_foundation::NSString;
use objc2_local_authentication::LABiometryType;
use objc2_local_authentication::LAContext;
use objc2_local_authentication::LAPolicy;
use security_framework::base::Error as SecurityError;
use security_framework::item::ItemClass;
use security_framework::item::ItemSearchOptions;
use security_framework::item::KeyClass;
use security_framework::item::Limit;
use security_framework::item::Location;
use security_framework::item::Reference;
use security_framework::item::SearchResult;
use security_framework::key::Algorithm;
use security_framework::key::GenerateKeyOptions;
use security_framework::key::KeyType;
use security_framework::key::SecKey;
use security_framework::key::Token;
use security_framework_sys::base::errSecItemNotFound;

pub(crate) struct NativeProvider {
    pub(crate) namespace: UserVerificationKeyNamespace,
}

pub(crate) fn device_supported() -> bool {
    let context = noninteractive_context();
    // canEvaluatePolicy populates biometryType even when enrollment or temporary lockout
    // prevents authentication. Advertise hardware support independently of current readiness.
    unsafe {
        let _ = context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics);
        context.biometryType() == LABiometryType::TouchID
    }
}

impl UserVerificationProvider for NativeProvider {
    fn status(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationStatus, UserVerificationError> {
        let _lock = LifecycleLock::acquire(&self.namespace, guard)?;
        let context = noninteractive_context();
        let credential = find_key(&self.namespace.label, &context, KeyUse::Protected)?
            .as_ref()
            .map(key_info)
            .transpose()?;
        let unavailable = if credential.is_none() {
            Some((
                UserVerificationUnavailableReason::CredentialMissing,
                "no user-verification credential has been created".to_string(),
            ))
        } else {
            match check_biometrics(&context) {
                Ok(()) => None,
                Err(UserVerificationError::Unavailable { reason, message }) => {
                    Some((reason, message))
                }
                Err(error) => return Err(error),
            }
        };
        let (unavailable_reason, unavailable_message) = match unavailable {
            Some((reason, message)) => (Some(reason), Some(message)),
            None => (None, None),
        };
        guard.check()?;
        Ok(UserVerificationStatus {
            credential,
            unavailable_reason,
            unavailable_message,
        })
    }

    fn ensure_key(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationKeyCreation, UserVerificationError> {
        let _lock = LifecycleLock::acquire(&self.namespace, guard)?;
        let context = noninteractive_context();
        let existing = find_key(&self.namespace.label, &context, KeyUse::Protected)?;
        guard.check()?;
        let (key, created) = match existing {
            Some(key) => (key, false),
            None => (create_key(&self.namespace.label)?, true),
        };
        let credential = key_info(&key)?;
        guard.check()?;
        Ok(UserVerificationKeyCreation {
            created,
            credential,
        })
    }

    fn delete(
        &self,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationKeyDeletion, UserVerificationError> {
        let _lock = LifecycleLock::acquire(&self.namespace, guard)?;
        let context = noninteractive_context();
        // The credential ID is optional information for the caller. A colliding key may
        // have no P-256 public key, or its lookup may fail because authentication is
        // unavailable; neither should prevent removing this namespace's key items.
        let credential = find_key(&self.namespace.label, &context, KeyUse::Deletion)
            .ok()
            .flatten()
            .and_then(|key| key_info(&key).ok());
        guard.check()?;
        let mut options = ItemSearchOptions::new();
        // GenerateKeyOptions persists both halves on macOS. Remove all key items under
        // this exact namespace, including public keys left by an interrupted deletion.
        options
            .ignore_legacy_keychains()
            .class(ItemClass::key())
            .label(&self.namespace.label);
        match options.delete() {
            Ok(()) => {}
            Err(error) if error.code() == errSecItemNotFound => {}
            Err(error) => return Err(keychain_error(error)),
        }
        guard.check()?;
        Ok(UserVerificationKeyDeletion {
            deleted_credential_id: credential.map(|key| key.credential_id),
        })
    }

    fn verify(
        &self,
        request: &UserVerificationRequest,
        guard: &UserVerificationRequestGuard,
    ) -> Result<UserVerificationProof, UserVerificationError> {
        guard.check()?;
        if request.challenge.is_empty()
            || request.challenge.len() > 4096
            || request.title.is_empty()
            || request.title.len() > 256
            || request.description.len() > 4096
        {
            return Err(UserVerificationError::Failed {
                reason: UserVerificationFailureReason::ProviderError,
                message: "invalid user-verification challenge or display text".to_string(),
            });
        }
        let _lock = LifecycleLock::acquire(&self.namespace, guard)?;
        // A fresh context for every signature prevents authentication reuse between actions.
        let context = unsafe { LAContext::new() };
        let reason = if request.description.is_empty() {
            request.title.clone()
        } else {
            format!("{}\n\n{}", request.title, request.description)
        };
        // The keychain query retains this context and uses its text for the signing prompt.
        unsafe {
            context.setLocalizedFallbackTitle(Some(&NSString::from_str("")));
            context.setLocalizedReason(&NSString::from_str(&reason));
        }
        check_biometrics(&context)?;
        guard.check()?;
        let key =
            find_key(&self.namespace.label, &context, KeyUse::Protected)?.ok_or_else(|| {
                UserVerificationError::Unavailable {
                    reason: UserVerificationUnavailableReason::CredentialMissing,
                    message: "no user-verification credential has been created".to_string(),
                }
            })?;
        let credential = key_info(&key)?;
        guard.check()?;
        let signature = run_with_cancellation(
            guard,
            || {
                key.create_signature(
                    Algorithm::ECDSASignatureMessageX962SHA256,
                    &request.challenge,
                )
                .map_err(|error| error::classify(&error.domain().to_string(), error.code() as i64))
            },
            || {
                // LAContext remains on this thread. Invalidation cancels its pending
                // keychain authentication, allowing the signer and lifecycle lock to exit.
                unsafe { context.invalidate() };
            },
        );
        guard.check()?;
        Ok(UserVerificationProof {
            credential_id: credential.credential_id,
            signature: URL_SAFE_NO_PAD.encode(signature?),
        })
    }
}

fn noninteractive_context() -> Retained<LAContext> {
    // Each context is confined to this blocking operation and never shared across threads.
    unsafe {
        let context = LAContext::new();
        context.setInteractionNotAllowed(/*interaction_not_allowed*/ true);
        context
    }
}

fn check_biometrics(context: &LAContext) -> Result<(), UserVerificationError> {
    // canEvaluatePolicy only checks availability; the protected key triggers authentication.
    unsafe { context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics) }
        .map_err(|error| error::classify(&error.domain().to_string(), error.code() as i64))
}

enum KeyUse {
    Protected,
    Deletion,
}

fn find_key(
    label: &str,
    context: &LAContext,
    key_use: KeyUse,
) -> Result<Option<SecKey>, UserVerificationError> {
    // Security accepts an LAContext object as kSecUseAuthenticationContext. Wrapping under
    // the get rule retains it, and ItemSearchOptions owns that retain for the query lifetime.
    let authentication = unsafe { CFType::wrap_under_get_rule(std::ptr::from_ref(context).cast()) };
    let mut options = ItemSearchOptions::new();
    options
        .ignore_legacy_keychains()
        .key_class(KeyClass::private())
        .label(label)
        .local_authentication_context(Some(authentication))
        .load_refs(true)
        .limit(Limit::Max(1));
    match options.search() {
        Ok(results) => {
            let key = results.into_iter().find_map(|result| match result {
                SearchResult::Ref(Reference::Key(key)) => Some(key),
                _ => None,
            });
            if let Some(key) = &key
                && matches!(key_use, KeyUse::Protected)
            {
                key_protection::validate(&key.attributes())?;
            }
            Ok(key)
        }
        Err(error) if error.code() == errSecItemNotFound => Ok(None),
        Err(error) => Err(keychain_error(error)),
    }
}

fn create_key(label: &str) -> Result<SecKey, UserVerificationError> {
    let access = key_protection::access_control()?;
    let mut options = GenerateKeyOptions::default();
    options
        .set_key_type(KeyType::ec_sec_prime_random())
        .set_size_in_bits(256)
        .set_label(label)
        .set_token(Token::SecureEnclave)
        .set_location(Location::DataProtectionKeychain)
        .set_access_control(access);
    let key = SecKey::new(&options)
        .map_err(|error| error::classify(&error.domain().to_string(), error.code() as i64))?;
    key_protection::validate(&key.attributes())?;
    Ok(key)
}

fn key_info(key: &SecKey) -> Result<UserVerificationKeyInfo, UserVerificationError> {
    let bytes = key
        .public_key()
        .and_then(|key| key.external_representation())
        .ok_or_else(|| UserVerificationError::Failed {
            reason: UserVerificationFailureReason::ProviderError,
            message: "could not export the user-verification public key".to_string(),
        })?;
    UserVerificationKeyInfo::from_sec1_public_key(&bytes)
}

fn keychain_error(error: SecurityError) -> UserVerificationError {
    error::classify("NSOSStatusErrorDomain", i64::from(error.code()))
}
