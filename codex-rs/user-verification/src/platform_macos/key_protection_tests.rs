//! Reject keys without the required Secure Enclave key attributes.

use super::*;
use core_foundation::base::CFType;
use core_foundation::dictionary::CFMutableDictionary;
use core_foundation::string::CFString;
use pretty_assertions::assert_eq;

fn protected_attributes() -> CFMutableDictionary<CFString, CFType> {
    unsafe {
        CFMutableDictionary::from_CFType_pairs(&[
            (
                CFString::wrap_under_get_rule(kSecAttrTokenID),
                CFString::wrap_under_get_rule(kSecAttrTokenIDSecureEnclave).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrKeyClass),
                CFString::wrap_under_get_rule(kSecAttrKeyClassPrivate).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrKeyType),
                CFString::wrap_under_get_rule(kSecAttrKeyTypeECSECPrimeRandom).as_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrKeySizeInBits),
                CFNumber::from(/*value*/ 256).as_CFType(),
            ),
        ])
    }
}

#[test]
fn reuse_requires_secure_enclave_key_attributes() {
    let attributes = protected_attributes();
    assert_eq!(validate(&attributes.to_immutable().to_untyped()), Ok(()));
    let alternatives = unsafe {
        [
            (kSecAttrTokenID, CFString::new("software").as_CFType()),
            (kSecAttrKeyClass, CFString::new("public").as_CFType()),
            (kSecAttrKeyType, CFString::new("RSA").as_CFType()),
            (
                kSecAttrKeySizeInBits,
                CFNumber::from(/*value*/ 384).as_CFType(),
            ),
        ]
    };
    for (attribute, value) in alternatives {
        let mut attributes = protected_attributes();
        attributes.set(unsafe { CFString::wrap_under_get_rule(attribute) }, value);
        assert_eq!(
            validate(&attributes.to_immutable().to_untyped()),
            Err(UserVerificationError::Unavailable {
                reason: UserVerificationUnavailableReason::ProviderUnavailable,
                message: "the local credential has incompatible key protection; delete it before enrolling again"
                    .to_string(),
            })
        );
    }
}
