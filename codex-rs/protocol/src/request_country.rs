//! Trusted runtime country for the current Responses request.

use serde::Serialize;
use serde::Serializer;

/// Country selected by a trusted host. The default leaves provenance to the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestCountry(Country);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Country {
    #[default]
    Omitted,
    Unknown,
    Known(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("request country must contain exactly two ASCII letters")]
pub struct InvalidRequestCountry;

impl RequestCountry {
    /// Explicitly unknown country. The server must not fall back to request provenance.
    pub const UNKNOWN: Self = Self(Country::Unknown);

    /// Normalizes a two-letter country code. The special code `XX` is explicit unknown.
    pub fn from_iso_code(code: &str) -> Result<Self, InvalidRequestCountry> {
        let code = code.trim();
        if code.len() != 2 || !code.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(InvalidRequestCountry);
        }
        let code = code.to_ascii_uppercase();
        if code == "XX" {
            return Ok(Self::UNKNOWN);
        }
        Ok(Self(Country::Known(code)))
    }

    pub fn known_country(&self) -> Option<&str> {
        match &self.0 {
            Country::Known(code) => Some(code),
            Country::Omitted | Country::Unknown => None,
        }
    }

    pub fn is_omitted(&self) -> bool {
        matches!(self.0, Country::Omitted)
    }
}

impl Serialize for RequestCountry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.known_country().serialize(serializer)
    }
}

#[cfg(test)]
#[path = "request_country_tests.rs"]
mod tests;
