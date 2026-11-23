use super::*;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn press(state: &mut LocalProviderState, code: KeyCode) -> Option<LocalProviderAction> {
    state.handle_key_event(&KeyEvent::new(code, KeyModifiers::NONE))
}

fn press_str(state: &mut LocalProviderState, text: &str) {
    for ch in text.chars() {
        state.handle_key_event(&KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
}

fn fill_form(base_url: &str, env_key: &str, model: &str) -> LocalProviderState {
    let mut state = LocalProviderState::new();
    press_str(&mut state, base_url);
    state.selected_field = 1;
    press_str(&mut state, env_key);
    state.selected_field = 2;
    press_str(&mut state, model);
    state
}

fn rendered_edits(edits: &[codex_app_server_protocol::ConfigEdit]) -> Vec<(String, String)> {
    edits
        .iter()
        .map(|edit| (edit.key_path.clone(), edit.value.to_string()))
        .collect()
}

#[test]
fn validate_rejects_empty_base_url() {
    let state = LocalProviderState::new();
    assert!(state.validate().is_err());
}

#[test]
fn validate_rejects_non_http_url() {
    let mut state = LocalProviderState::new();
    press_str(&mut state, "ftp://server/v1");
    state.selected_field = 2;
    press_str(&mut state, "llama3.2");
    assert!(state.validate().is_err());
}

#[test]
fn validate_rejects_credential_bearing_url() {
    for base_url in [
        "http://user:pass@localhost:11434/v1",
        "https://token@endpoint.example/v1",
    ] {
        let state = fill_form(base_url, "", "llama3.2");
        let message = state.validate().expect_err("credentials must be rejected");
        assert!(
            message.contains("credentials"),
            "unexpected error: {message}"
        );
    }
}

#[test]
fn validate_rejects_invalid_env_key_names() {
    for env_key in ["9BAD", "MY KEY", "MY-KEY", "PATH/TO", "KEY=VALUE"] {
        let state = fill_form("http://localhost:11434/v1", env_key, "llama3.2");
        let message = state
            .validate()
            .expect_err("invalid env var name must be rejected");
        assert!(
            message.contains("environment variable"),
            "unexpected error: {message}"
        );
    }
}

#[test]
fn validate_accepts_full_form_without_env_key() {
    let mut state = LocalProviderState::new();
    press_str(&mut state, "http://localhost:11434/v1");
    state.selected_field = 2;
    press_str(&mut state, "llama3.2");
    let config = state.validate().expect("valid form");
    assert_eq!(config.base_url, "http://localhost:11434/v1");
    assert_eq!(config.env_key, None);
    assert_eq!(config.model, "llama3.2");
}

#[test]
fn validate_accepts_underscore_prefixed_env_key() {
    let state = fill_form("http://localhost:11434/v1", "_PRIVATE_KEY", "llama3.2");
    let config = state.validate().expect("valid form");
    assert_eq!(config.env_key.as_deref(), Some("_PRIVATE_KEY"));
}

#[test]
fn enter_advances_then_configures() {
    let mut state = LocalProviderState::new();
    press_str(&mut state, "http://localhost:11434/v1");
    // Enter on a filled field advances to the next one.
    assert!(matches!(press(&mut state, KeyCode::Enter), None));
    assert_eq!(state.selected_field, 1);
    press_str(&mut state, "OLLAMA_API_KEY");
    assert!(matches!(press(&mut state, KeyCode::Enter), None));
    assert_eq!(state.selected_field, 2);
    press_str(&mut state, "llama3.2");
    let action = press(&mut state, KeyCode::Enter);
    match action {
        Some(LocalProviderAction::Configure(config)) => {
            assert_eq!(config.env_key.as_deref(), Some("OLLAMA_API_KEY"));
        }
        _ => panic!("expected Configure action"),
    }
}

#[test]
fn esc_returns_to_picker() {
    let mut state = LocalProviderState::new();
    assert!(matches!(
        press(&mut state, KeyCode::Esc),
        Some(LocalProviderAction::BackToAuth)
    ));
}

#[test]
fn edits_write_provider_and_model() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:11434/v1".to_string(),
        env_key: Some("OLLAMA_API_KEY".to_string()),
        model: "llama3.2".to_string(),
    };
    let edits = local_provider_edits("local", &config);
    assert_eq!(
        rendered_edits(&edits),
        vec![
            (
                "model_providers.local.name".to_string(),
                "\"Local / custom endpoint\"".to_string()
            ),
            (
                "model_providers.local.base_url".to_string(),
                "\"http://localhost:11434/v1\"".to_string()
            ),
            (
                "model_providers.local.wire_api".to_string(),
                "\"chat\"".to_string()
            ),
            (
                "model_providers.local.requires_openai_auth".to_string(),
                "false".to_string()
            ),
            ("model_provider".to_string(), "\"local\"".to_string()),
            ("model".to_string(), "\"llama3.2\"".to_string()),
            (
                "model_providers.local.env_key".to_string(),
                "\"OLLAMA_API_KEY\"".to_string()
            ),
        ]
    );
}

/// Regression: leaving the env-key field empty must not clear an env_key the
/// user already configured on the entry.
#[test]
fn edits_without_env_key_emit_no_env_key_edit() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:11434/v1".to_string(),
        env_key: None,
        model: "llama3.2".to_string(),
    };
    let edits = local_provider_edits("local", &config);
    assert!(
        edits
            .iter()
            .all(|edit| !edit.key_path.ends_with(".env_key")),
        "empty env-key field must not produce an env_key edit: {:?}",
        rendered_edits(&edits)
    );
}

/// Regression: a collision never rewrites `model_providers.local`; the form is
/// saved under a fresh `local-N` id instead.
#[test]
fn collision_writes_new_provider_id_not_existing_entry() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: Some("LMSTUDIO_API_KEY".to_string()),
        model: "qwen2.5".to_string(),
    };
    let new_id = next_local_provider_id(&["local".to_string(), "openai".to_string()]);
    assert_eq!(new_id, "local-2");

    let edits = local_provider_edits(&new_id, &config);
    let rendered = rendered_edits(&edits);
    assert!(
        rendered
            .iter()
            .all(|(key, _)| !key.starts_with("model_providers.local.")),
        "must not touch the existing local entry: {rendered:?}"
    );
    assert!(rendered.contains(&(
        "model_providers.local-2.base_url".to_string(),
        "\"http://localhost:1234/v1\"".to_string()
    )));
    assert!(rendered.contains(&("model_provider".to_string(), "\"local-2\"".to_string())));
}

/// Regression: adopting the existing entry only selects provider/model and
/// never rewrites the entry's own fields.
#[test]
fn adopt_existing_preserves_provider_entry() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:11434/v1".to_string(),
        env_key: None,
        model: "llama3.2".to_string(),
    };
    let edits = adopt_existing_provider_edits("local", &config);
    assert_eq!(
        rendered_edits(&edits),
        vec![
            ("model_provider".to_string(), "\"local\"".to_string()),
            ("model".to_string(), "\"llama3.2\"".to_string()),
        ]
    );
}

#[test]
fn adopt_existing_updates_env_key_only_when_typed() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:11434/v1".to_string(),
        env_key: Some("NEW_KEY".to_string()),
        model: "llama3.2".to_string(),
    };
    let edits = adopt_existing_provider_edits("local", &config);
    let rendered = rendered_edits(&edits);
    assert_eq!(rendered.len(), 3);
    assert!(rendered.contains(&(
        "model_providers.local.env_key".to_string(),
        "\"NEW_KEY\"".to_string()
    )));
}

#[test]
fn next_local_provider_id_skips_taken_ids() {
    assert_eq!(next_local_provider_id(&[]), "local");
    assert_eq!(
        next_local_provider_id(&["local".to_string(), "local-2".to_string()]),
        "local-3"
    );
    // Case-insensitive: `Local` still blocks the plain id.
    assert_eq!(next_local_provider_id(&["Local".to_string()]), "local-2");
}

#[test]
fn collision_picker_confirms_selected_resolution() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
        model: "qwen2.5".to_string(),
    };
    let mut picker = LocalProviderCollisionState::new(
        config,
        "Existing 'Local' provider".to_string(),
        "local-2".to_string(),
    );
    // The non-destructive "create new" choice is highlighted by default.
    assert!(matches!(
        picker.handle_key_event(&KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        Some(LocalProviderCollisionAction::Resolve(
            LocalProviderResolution::CreateNew
        ))
    ));
    // '1' picks "use existing"; Esc returns to the form.
    assert!(matches!(
        picker.handle_key_event(&KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
        Some(LocalProviderCollisionAction::Resolve(
            LocalProviderResolution::UseExisting
        ))
    ));
    assert!(matches!(
        picker.handle_key_event(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        Some(LocalProviderCollisionAction::BackToForm)
    ));
}

#[test]
fn render_shows_fields_and_footer() {
    let state = LocalProviderState::new();
    let backend = TestBackend::new(80, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| state.render(frame.area(), frame.buffer_mut(), None))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let visible = (0..16)
        .map(|y| {
            (0..80)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(visible.contains("Use a local / custom provider"));
    assert!(visible.contains("Base URL"));
    assert!(visible.contains("Model"));
    assert!(visible.contains("esc"));
}

#[test]
fn collision_render_shows_existing_and_choices() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
        model: "qwen2.5".to_string(),
    };
    let picker = LocalProviderCollisionState::new(
        config,
        "Existing 'Local / custom endpoint' provider points at http://localhost:11434/v1."
            .to_string(),
        "local-2".to_string(),
    );
    let backend = TestBackend::new(100, 12);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| picker.render(frame.area(), frame.buffer_mut()))
        .unwrap();
    let buf = terminal.backend().buffer().clone();
    let visible = (0..12)
        .map(|y| {
            (0..100)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(visible.contains("already exists"));
    assert!(visible.contains("local-2"));
    assert!(visible.contains("qwen2.5"));
}
