#[cfg(test)]
pub(super) use codex_config::schema::canonicalize;
#[cfg(test)]
pub(super) use codex_config::schema::config_schema_json;
#[cfg(test)]
pub(super) use codex_config::schema::write_config_schema;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
