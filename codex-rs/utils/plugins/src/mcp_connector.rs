use codex_app_server_protocol::AppInfo;

const DISALLOWED_CONNECTOR_IDS: &[&str] = &[
    "asdk_app_6938a94a61d881918ef32cb999ff937c",
    "connector_2b0a9009c9c64bf9933a3dae3f2b1254",
    "connector_3f8d1a79f27c4c7ba1a897ab13bf37dc",
    "connector_68de829bf7648191acd70a907364c67c",
    "connector_68e004f14af881919eb50893d3d9f523",
    "connector_69272cb413a081919685ec3c88d1744e",
];
const FIRST_PARTY_CHAT_DISALLOWED_CONNECTOR_IDS: &[&str] =
    &["connector_0f9c9d4592e54d0a9a12b3f44a1e2010"];
const DISALLOWED_CONNECTOR_PREFIX: &str = "connector_openai_";
const DEFAULT_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR: &str = "CODEX_INTERNAL_ORIGINATOR_OVERRIDE";

pub fn is_connector_id_allowed(connector_id: &str) -> bool {
    let originator_value = std::env::var(CODEX_INTERNAL_ORIGINATOR_OVERRIDE_ENV_VAR)
        .ok()
        .unwrap_or(DEFAULT_ORIGINATOR.to_string());
    is_connector_id_allowed_for_originator(connector_id, &originator_value)
}

fn is_connector_id_allowed_for_originator(connector_id: &str, originator_value: &str) -> bool {
    let disallowed_connector_ids =
        if matches!(originator_value, "codex_atlas" | "codex_chatgpt_desktop") {
            FIRST_PARTY_CHAT_DISALLOWED_CONNECTOR_IDS
        } else {
            DISALLOWED_CONNECTOR_IDS
        };

    !connector_id.starts_with(DISALLOWED_CONNECTOR_PREFIX)
        && !disallowed_connector_ids.contains(&connector_id)
}

pub fn sanitize_name(name: &str) -> String {
    sanitize_slug(name).replace("-", "_")
}

pub fn connector_display_label(connector: &AppInfo) -> String {
    format_connector_label(&connector.name, &connector.id)
}

pub fn connector_mention_slug(connector: &AppInfo) -> String {
    sanitize_slug(&connector_display_label(connector))
}

fn sanitize_slug(name: &str) -> String {
    let mut normalized = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push('-');
        }
    }
    let normalized = normalized.trim_matches('-');
    if normalized.is_empty() {
        "app".to_string()
    } else {
        normalized.to_string()
    }
}

fn format_connector_label(name: &str, _id: &str) -> String {
    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_originator_uses_chat_specific_disallow_list() {
        assert!(!is_connector_id_allowed_for_originator(
            "connector_0f9c9d4592e54d0a9a12b3f44a1e2010",
            "codex_atlas"
        ));
        assert!(is_connector_id_allowed_for_originator(
            "connector_2b0a9009c9c64bf9933a3dae3f2b1254",
            "codex_atlas"
        ));
    }

    #[test]
    fn cli_originator_uses_cli_disallow_list() {
        assert!(!is_connector_id_allowed_for_originator(
            "connector_2b0a9009c9c64bf9933a3dae3f2b1254",
            DEFAULT_ORIGINATOR
        ));
        assert!(is_connector_id_allowed_for_originator(
            "connector_0f9c9d4592e54d0a9a12b3f44a1e2010",
            DEFAULT_ORIGINATOR
        ));
    }
}
