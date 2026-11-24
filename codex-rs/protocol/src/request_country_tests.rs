#![cfg(test)]

use super::*;

#[test]
fn request_country_rejects_malformed_values() {
    for malformed in ["", "D", "USA", "D1", "T1", "DÉ", "D E", "DE\nX"] {
        assert_eq!(
            RequestCountry::from_iso_code(malformed),
            Err(InvalidRequestCountry),
        );
    }
}
