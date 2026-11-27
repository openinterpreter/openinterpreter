use super::*;

use crate::legacy_core::config::ConfigBuilder;
use crate::onboarding::auth::SignInState;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_client::InProcessAppServerClient;
use color_eyre::eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::RwLock;
use tempfile::TempDir;

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
        "local".to_string(),
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
        "local".to_string(),
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

/// A user config that already defines `model_providers.local`, which is what
/// forces the collision picker.
const EXISTING_LOCAL_PROVIDER: &str = r#"
[model_providers.local]
name = "Local / custom endpoint"
base_url = "http://localhost:11434/v1"
wire_api = "chat"
requires_openai_auth = false
"#;

/// Embedded app-server harness for the persistence flow: a temp `codex_home`
/// with a `config.toml`, the flow pointed at it, and a second request handle
/// that stands in for another writer of the same config.
struct FlowHarness {
    flow: LocalProviderFlow,
    other_writer: AppServerRequestHandle,
    config_path: PathBuf,
    _app_server: InProcessAppServerClient,
    _codex_home: TempDir,
}

async fn flow_harness(config_toml: &str) -> Result<FlowHarness> {
    let codex_home = TempDir::new()?;
    let config_path = codex_home.path().join("config.toml");
    std::fs::write(&config_path, config_toml)?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await?;
    let app_server = crate::tests::start_test_embedded_app_server(config).await?;
    let request_handle = AppServerRequestHandle::InProcess(app_server.request_handle());
    Ok(FlowHarness {
        flow: LocalProviderFlow {
            request_handle: request_handle.clone(),
            sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
            error: Arc::new(RwLock::new(None)),
        },
        other_writer: request_handle,
        config_path,
        _app_server: app_server,
        _codex_home: codex_home,
    })
}

/// The form values currently held by the sign-in state, when it is showing the
/// local-provider form.
fn form_values(state: &SignInState) -> Option<Vec<String>> {
    match state {
        SignInState::LocalProvider(form) => Some(form.values.to_vec()),
        _ => None,
    }
}

fn entered_values(config: &LocalProviderConfig) -> Vec<String> {
    vec![
        config.base_url.clone(),
        config.env_key.clone().unwrap_or_default(),
        config.model.clone(),
    ]
}

/// Regression: backing out of the collision picker returns to the completed
/// form, not to an empty one.
#[test]
fn collision_picker_keeps_completed_form_for_back_navigation() {
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: Some("LMSTUDIO_API_KEY".to_string()),
        model: "qwen2.5".to_string(),
    };
    let picker = LocalProviderCollisionState::new(
        config.clone(),
        "local".to_string(),
        "Existing 'Local / custom endpoint' provider points at http://localhost:11434/v1."
            .to_string(),
        "local-2".to_string(),
    );

    assert_eq!(picker.form.values.to_vec(), entered_values(&config));
    assert_eq!(picker.form.selected_field, 2);
}

/// Regression: a rejected write must keep the entered values on screen and must
/// not touch the config — the write is pinned to the version it was derived
/// from, so a config that changed in the meantime is never overwritten.
#[tokio::test]
async fn stale_config_version_write_keeps_entries_and_writes_nothing() -> Result<()> {
    let harness = flow_harness("model = \"gpt-old\"\n").await?;
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: Some("LMSTUDIO_API_KEY".to_string()),
        model: "qwen2.5".to_string(),
    };

    harness
        .flow
        .persist(
            LocalProviderState::from_config(&config),
            "sha256:stale".to_string(),
            local_provider_edits("local-2", &config),
        )
        .await;

    assert_eq!(
        form_values(&harness.flow.sign_in_state.read().unwrap()),
        Some(entered_values(&config))
    );
    let error = harness
        .flow
        .error
        .read()
        .unwrap()
        .clone()
        .unwrap_or_default();
    assert!(
        error.contains("configuration changed"),
        "unexpected error: {error}"
    );
    let persisted = std::fs::read_to_string(&harness.config_path)?;
    assert!(
        !persisted.contains("local-2"),
        "a rejected write must not create a provider: {persisted}"
    );
    Ok(())
}

/// Regression: the `local-N` id proposed by the picker is revalidated right
/// before the write. An id claimed by another writer in between is reported and
/// neither that writer's entry nor the submitted form is written.
#[tokio::test]
async fn new_provider_id_claimed_between_read_and_write_is_not_overwritten() -> Result<()> {
    let harness = flow_harness(EXISTING_LOCAL_PROVIDER).await?;
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
        model: "qwen2.5".to_string(),
    };

    harness.flow.save(config.clone()).await;
    let collision = match &*harness.flow.sign_in_state.read().unwrap() {
        SignInState::LocalProviderCollision(collision) => collision.clone(),
        _ => panic!("expected the collision picker"),
    };
    assert_eq!(collision.new_provider_id, "local-2");

    // Another writer claims the id the picker allocated before this form is
    // written.
    let other_config = LocalProviderConfig {
        base_url: "http://localhost:8080/v1".to_string(),
        env_key: None,
        model: "other-model".to_string(),
    };
    crate::config_update::write_config_batch(
        harness.other_writer.clone(),
        local_provider_edits(&collision.new_provider_id, &other_config),
    )
    .await?;

    harness
        .flow
        .resolve_collision(collision, LocalProviderResolution::CreateNew)
        .await;

    let error = harness
        .flow
        .error
        .read()
        .unwrap()
        .clone()
        .unwrap_or_default();
    assert!(
        error.contains("was created by another configuration change"),
        "unexpected error: {error}"
    );
    assert_eq!(
        form_values(&harness.flow.sign_in_state.read().unwrap()),
        Some(entered_values(&config))
    );
    let persisted = std::fs::read_to_string(&harness.config_path)?;
    assert!(
        !persisted.contains("http://localhost:1234/v1"),
        "the submitted form must not be written over the claimed id: {persisted}"
    );
    assert!(
        persisted.contains("http://localhost:8080/v1"),
        "the other writer's entry must survive: {persisted}"
    );
    Ok(())
}

/// Regression: a local provider stored under a non-lowercase id still counts
/// as a collision — `is_provider_id_free` treats `LOCAL` and `local` as the
/// same id, so detection and adoption must use the same semantics. The picker
/// has to surface the entry and adopt the *actual* key, not the canonical
/// lowercase one, or `model_provider` would point at an id that does not exist.
#[tokio::test]
async fn mixed_case_local_provider_collides_and_adopts_actual_key() -> Result<()> {
    let harness = flow_harness(
        "[model_providers.LOCAL]\n\
         name = \"Local / custom endpoint\"\n\
         base_url = \"http://localhost:11434/v1\"\n\
         wire_api = \"chat\"\n",
    )
    .await?;
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: Some("LMSTUDIO_API_KEY".to_string()),
        model: "qwen2.5".to_string(),
    };

    harness.flow.save(config.clone()).await;
    let collision = match &*harness.flow.sign_in_state.read().unwrap() {
        SignInState::LocalProviderCollision(collision) => collision.clone(),
        _ => panic!("expected the collision picker for `LOCAL`"),
    };
    assert_eq!(collision.existing_provider_id, "LOCAL");
    // `local-2`, not `local`: the allocator must consider `LOCAL` taken under
    // the same case-insensitive rule detection used.
    assert_eq!(collision.new_provider_id, "local-2");

    harness
        .flow
        .resolve_collision(collision, LocalProviderResolution::UseExisting)
        .await;

    assert!(matches!(
        &*harness.flow.sign_in_state.read().unwrap(),
        SignInState::LocalProviderConfigured
    ));
    let persisted = std::fs::read_to_string(&harness.config_path)?;
    assert!(
        persisted.contains("model_provider = \"LOCAL\""),
        "adoption must select the actual `LOCAL` key, not `local`: {persisted}"
    );
    assert!(
        !persisted.contains("[model_providers.local]"),
        "adopting `LOCAL` must not create a duplicate `local` entry: {persisted}"
    );
    Ok(())
}

/// Regression: when the config read finds no user layer at all (`config.toml`
/// absent), `user_config_version` is `None`. Pinning the write to `None` would
/// skip the version check entirely and let a first write silently replace a
/// `local` entry a concurrent writer created after the read. Instead the
/// snapshot substitutes the empty-layer version as the expected version, so a
/// file created in between is a version conflict — never an overwrite — and a
/// still-absent file still lets the create through.
#[tokio::test]
async fn first_write_race_does_not_overwrite_concurrent_local() -> Result<()> {
    let harness = flow_harness("model = \"gpt-old\"\n").await?;
    // Remove `config.toml` so the snapshot read reports no user layer; the
    // snapshot falls back to the empty-layer version instead of `None`.
    std::fs::remove_file(&harness.config_path)?;
    let snapshot = ProviderSnapshot::load(&harness.flow.request_handle).await?;
    assert!(snapshot.local_summary.is_none());
    assert_eq!(
        snapshot.expected_version,
        empty_user_layer_version(),
        "a read with no user layer must pin to the empty-layer version"
    );

    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
        model: "qwen2.5".to_string(),
    };
    // The other writer wins the race: it creates `config.toml` and claims
    // `local` between the snapshot read and this form's write.
    let other_config = LocalProviderConfig {
        base_url: "http://localhost:8080/v1".to_string(),
        env_key: None,
        model: "other-model".to_string(),
    };
    crate::config_update::write_config_batch(
        harness.other_writer.clone(),
        local_provider_edits("local", &other_config),
    )
    .await?;

    // Replaying the stale write is a conflict: the winning entry is preserved,
    // the form's `local` is not written over it, and the typed values survive.
    harness
        .flow
        .persist(
            LocalProviderState::from_config(&config),
            snapshot.expected_version,
            local_provider_edits("local", &config),
        )
        .await;

    let error = harness
        .flow
        .error
        .read()
        .unwrap()
        .clone()
        .unwrap_or_default();
    assert!(
        error.contains("configuration changed"),
        "expected a version-conflict error, got: {error}"
    );
    assert_eq!(
        form_values(&harness.flow.sign_in_state.read().unwrap()),
        Some(entered_values(&config))
    );
    let persisted = std::fs::read_to_string(&harness.config_path)?;
    assert!(
        persisted.contains("http://localhost:8080/v1"),
        "the winning writer's entry must survive: {persisted}"
    );
    assert!(
        !persisted.contains("http://localhost:1234/v1"),
        "the stale form must not overwrite `local`: {persisted}"
    );
    Ok(())
}

/// Regression: the empty-layer fallback only guards the create when a writer
/// raced in — when `config.toml` is still absent the version matches the layer
/// the server creates on the fly, so a genuinely first write still succeeds.
#[tokio::test]
async fn first_write_succeeds_when_config_still_absent() -> Result<()> {
    let harness = flow_harness("model = \"gpt-old\"\n").await?;
    std::fs::remove_file(&harness.config_path)?;
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: None,
        model: "qwen2.5".to_string(),
    };

    harness.flow.save(config).await;

    assert!(matches!(
        &*harness.flow.sign_in_state.read().unwrap(),
        SignInState::LocalProviderConfigured
    ));
    let persisted = std::fs::read_to_string(&harness.config_path)?;
    assert!(
        persisted.contains("http://localhost:1234/v1"),
        "the first write must land: {persisted}"
    );
    Ok(())
}

/// Regression: a failing config read keeps the entered values instead of
/// replacing the completed form with an empty one.
#[tokio::test]
async fn config_read_failure_keeps_entries() -> Result<()> {
    let harness = flow_harness("model = \"gpt-old\"\n").await?;
    let config = LocalProviderConfig {
        base_url: "http://localhost:1234/v1".to_string(),
        env_key: Some("LMSTUDIO_API_KEY".to_string()),
        model: "qwen2.5".to_string(),
    };
    // Break the user config so the pre-write read has to fail.
    std::fs::write(&harness.config_path, "not = a = valid = config\n")?;

    harness.flow.save(config.clone()).await;

    let error = harness
        .flow
        .error
        .read()
        .unwrap()
        .clone()
        .unwrap_or_default();
    assert!(
        error.contains("Failed to check existing provider configuration"),
        "unexpected error: {error}"
    );
    assert_eq!(
        form_values(&harness.flow.sign_in_state.read().unwrap()),
        Some(entered_values(&config))
    );
    Ok(())
}
