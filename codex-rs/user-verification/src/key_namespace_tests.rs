//! Account-user identities share one service scope without exposing raw identifiers.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn namespaces_are_stable_and_separate_accounts() {
    let first = UserVerificationKeyNamespace::new("account-user-one");
    assert_eq!(first, UserVerificationKeyNamespace::new("account-user-one"));
    assert_ne!(first, UserVerificationKeyNamespace::new("account-user-two"));
    assert!(!first.label.contains("account-user-one"));
}
