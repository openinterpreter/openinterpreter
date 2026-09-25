//! Checks that the exported credential interoperates with DER-based service verification.

use super::*;
use p256::ecdsa::Signature;
use p256::ecdsa::SigningKey;
use p256::ecdsa::VerifyingKey;
use p256::ecdsa::signature::Signer as _;
use p256::ecdsa::signature::Verifier as _;
use p256::pkcs8::DecodePublicKey as _;
use pretty_assertions::assert_eq;

#[test]
fn exported_spki_verifies_the_exact_challenge_signature() {
    let key = SigningKey::from_bytes((&[7_u8; 32]).into()).expect("valid signing key");
    let info = UserVerificationKeyInfo::from_sec1_public_key(
        key.verifying_key()
            .to_encoded_point(/*compress*/ false)
            .as_bytes(),
    )
    .expect("encode public key");
    let der = URL_SAFE_NO_PAD.decode(&info.public_key).expect("base64url");
    let verifier = VerifyingKey::from_public_key_der(&der).expect("valid SPKI DER");
    let challenge = b"the exact server-issued challenge";
    let signature: Signature = key.sign(challenge);
    let signature = Signature::from_der(signature.to_der().as_bytes()).expect("DER signature");
    verifier
        .verify(challenge, &signature)
        .expect("valid signature");
    assert!(verifier.verify(b"different challenge", &signature).is_err());
    assert_eq!(
        info,
        UserVerificationKeyInfo {
            credential_id: URL_SAFE_NO_PAD.encode(Sha256::digest(&der)),
            algorithm: "ecdsaP256Sha256X962".to_string(),
            public_key: URL_SAFE_NO_PAD.encode(der),
        }
    );
}

#[test]
fn invalid_curve_point_is_rejected() {
    assert!(UserVerificationKeyInfo::from_sec1_public_key(&[4_u8; 65]).is_err());
}
