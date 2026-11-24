//! Opt-in checks of the persisted Secure Enclave signing and cancellation path.
#![cfg(target_os = "macos")]

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use codex_user_verification::UserVerificationCancellationReason;
use codex_user_verification::UserVerificationError;
use codex_user_verification::UserVerificationKeyNamespace;
use codex_user_verification::UserVerificationProvider;
use codex_user_verification::UserVerificationRequest;
use codex_user_verification::UserVerificationRequestGuard;
use codex_user_verification::platform_provider;
use p256::ecdsa::Signature;
use p256::ecdsa::VerifyingKey;
use p256::ecdsa::signature::Verifier as _;
use p256::pkcs8::DecodePublicKey as _;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

const TEST_IDENTITY: &str = "codex-user-verification-hardware-test";

struct TestKey(Arc<dyn UserVerificationProvider>);

impl Drop for TestKey {
    fn drop(&mut self) {
        let deleted = self.0.delete(&UserVerificationRequestGuard::default());
        if !std::thread::panicking() {
            assert!(deleted.is_ok(), "delete the test credential: {deleted:?}");
        }
    }
}

#[test]
#[ignore = "requires a signed test app, a physical Touch ID Mac, and user interaction; see README.md"]
fn persisted_key_signing_cancellation_and_retry() {
    let provider = platform_provider(UserVerificationKeyNamespace::new(TEST_IDENTITY));
    // This namespace is reserved for this test, including cleanup after a killed run.
    provider
        .delete(&UserVerificationRequestGuard::default())
        .expect("clear any previous test credential");
    if std::env::var_os("CODEX_HARDWARE_TEST_CLEANUP").is_some() {
        return;
    }
    let cleanup = TestKey(Arc::clone(&provider));
    let created = provider
        .ensure_key(&UserVerificationRequestGuard::default())
        .expect("create a persisted Secure Enclave key");
    assert!(created.created);
    let reloaded = platform_provider(UserVerificationKeyNamespace::new(TEST_IDENTITY))
        .ensure_key(&UserVerificationRequestGuard::default())
        .expect("reload the persisted key");
    assert!(!reloaded.created);
    assert_eq!(reloaded.credential, created.credential);
    let public_key = VerifyingKey::from_public_key_der(
        &URL_SAFE_NO_PAD
            .decode(&created.credential.public_key)
            .unwrap(),
    )
    .unwrap();
    let sign = |request: &UserVerificationRequest| {
        let proof = provider
            .verify(request, &UserVerificationRequestGuard::default())
            .expect("sign with the persisted key");
        assert_eq!(proof.credential_id, created.credential.credential_id);
        let signature =
            Signature::from_der(&URL_SAFE_NO_PAD.decode(proof.signature).unwrap()).unwrap();
        public_key.verify(&request.challenge, &signature).unwrap();
    };
    let mut request = UserVerificationRequest {
        challenge: b"Local hardware test. No purchase.".to_vec(),
        title: "Hardware test: authenticate".into(),
        description: "Use Touch ID once, then remove your finger.".into(),
    };
    eprintln!("Use Touch ID for the first signature.");
    sign(&request);

    request.title = "Hardware test: cancellation".into();
    request.description =
        "Leave Touch ID untouched; this prompt should close automatically.".into();
    eprintln!("Leave the next prompt untouched; cancellation follows in three seconds.");
    let guard = UserVerificationRequestGuard::default();
    let cancellation = guard.clone();
    let (finished, completion) = mpsc::channel();
    let timer = std::thread::spawn(move || {
        if completion.recv_timeout(Duration::from_secs(/* secs */ 3))
            == Err(mpsc::RecvTimeoutError::Timeout)
        {
            cancellation.cancel();
            if completion.recv_timeout(Duration::from_secs(/* secs */ 10))
                == Err(mpsc::RecvTimeoutError::Timeout)
            {
                eprintln!(
                    "Cancellation did not finish. Run with CODEX_HARDWARE_TEST_CLEANUP=1 to remove the test key."
                );
                std::process::exit(/* code */ 1);
            }
            true
        } else {
            false
        }
    });
    let result = provider.verify(&request, &guard);
    finished.send(()).unwrap();
    assert!(
        timer.join().unwrap(),
        "verification finished before cancellation"
    );
    assert!(
        matches!(
            result,
            Err(UserVerificationError::Cancelled {
                reason: UserVerificationCancellationReason::Interrupted,
                ..
            })
        ),
        "unexpected cancellation result: {result:?}"
    );

    request.challenge = b"Retry signing after cancellation.".to_vec();
    request.title = "Hardware test: retry signing".into();
    request.description = "Use Touch ID to sign again with the same stored key.".into();
    eprintln!("Use Touch ID for the retry.");
    sign(&request);
    drop(cleanup);
    assert!(
        provider
            .status(&UserVerificationRequestGuard::default())
            .unwrap()
            .credential
            .is_none()
    );
}
