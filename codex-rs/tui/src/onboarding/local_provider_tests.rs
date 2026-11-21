use super::*;

use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn press(state: &mut LocalProviderState, code: KeyCode) -> Option<LocalProviderAction> {
    state.handle_key_event(&KeyEvent::new(code, KeyModifiers::NONE))
}

fn press_str(state: &mut LocalProviderState, text: &str) {
    for ch in text.chars() {
        state.handle_key_event(&KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
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
fn enter_advances_then_configures() {
    let mut state = LocalProviderState::new();
    press_str(&mut state, "http://localhost:11434/v1");
    // Enter on a filled field advances to the next one.
    assert!(matches!(
        press(&mut state, KeyCode::Enter),
        None
    ));
    assert_eq!(state.selected_field, 1);
    press_str(&mut state, "OLLAMA_API_KEY");
    assert!(matches!(
        press(&mut state, KeyCode::Enter),
        None
    ));
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
    let edits = local_provider_edits(&config);
    let rendered: Vec<(String, String)> = edits
        .iter()
        .map(|edit| (edit.key_path.clone(), edit.value.to_string()))
        .collect();
    assert_eq!(
        rendered,
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
