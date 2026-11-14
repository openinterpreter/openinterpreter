use codex_config::CONFIG_TOML_FILE;
use codex_config::ConfigLayerStack;
use codex_config::TomlValue;
use codex_core::config::Config;
use codex_features::Feature;
use codex_hooks::HookListEntry;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::json;

/// Build the MCP reply for a loopback hook fixture; notifications have no body.
pub fn hook_mcp_reply(
    request: &serde_json::Value,
    server: &str,
    tools: &[&str],
    call: impl FnOnce(&serde_json::Value) -> serde_json::Value,
) -> Option<serde_json::Value> {
    let method = request["method"].as_str().expect("MCP method");
    let Some(id) = request.get("id") else {
        assert!(
            method.starts_with("notifications/"),
            "missing ID for {method}"
        );
        return None;
    };
    let result = match method {
        "initialize" => json!({
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": server, "version": "1.0.0"},
        }),
        "tools/list" => {
            json!({"tools":tools.iter().map(|name| json!({"name":name,"inputSchema":{"type":"object"}})).collect::<Vec<_>>()})
        }
        "tools/call" => call(&request["params"]),
        method => panic!("unexpected MCP request: {method}"),
    };
    Some(json!({"jsonrpc":"2.0", "id":id, "result":result}))
}

pub async fn mount_hook_mcp(
    server: &wiremock::MockServer,
    name: &'static str,
    tools: &'static [&'static str],
    respond: impl Fn(&serde_json::Value) -> serde_json::Value + Send + Sync + 'static,
) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/repl"))
        .respond_with(move |req: &wiremock::Request| {
            let request = serde_json::from_slice(&req.body).expect("MCP request");
            match hook_mcp_reply(&request, name, tools, &respond) {
                Some(reply) => wiremock::ResponseTemplate::new(200).set_body_json(reply),
                None => wiremock::ResponseTemplate::new(202),
            }
        })
        .mount(server)
        .await;
}

pub fn trust_discovered_hooks(config: &mut Config) {
    config
        .features
        .enable(Feature::CodexHooks)
        .expect("test config should allow feature update");

    let listed = codex_hooks::list_hooks(codex_hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(config.config_layer_stack.clone()),
        ..codex_hooks::HooksConfig::default()
    });
    assert!(
        !listed.hooks.is_empty(),
        "trusted hook fixture should discover at least one hook"
    );
    trust_hooks(config, listed.hooks);
}

pub fn trust_hooks(config: &mut Config, hooks: Vec<HookListEntry>) {
    config.config_layer_stack =
        trusted_config_layer_stack(&config.config_layer_stack, &config.codex_home, hooks);
}

pub fn trusted_config_layer_stack(
    config_layer_stack: &ConfigLayerStack,
    codex_home: &AbsolutePathBuf,
    hooks: Vec<HookListEntry>,
) -> ConfigLayerStack {
    let mut user_config = config_layer_stack
        .get_active_user_layer()
        .map(|layer| layer.config.clone())
        .unwrap_or_else(|| TomlValue::Table(Default::default()));
    let user_table = user_config
        .as_table_mut()
        .expect("user config should be a table");
    let hooks_table = user_table
        .entry("hooks")
        .or_insert_with(|| TomlValue::Table(Default::default()))
        .as_table_mut()
        .expect("hooks config should be a table");
    let state_table = hooks_table
        .entry("state")
        .or_insert_with(|| TomlValue::Table(Default::default()))
        .as_table_mut()
        .expect("hook state config should be a table");
    for hook in hooks {
        let mut hook_state = TomlValue::Table(Default::default());
        let hook_state_table = hook_state
            .as_table_mut()
            .expect("hook state should be a table");
        hook_state_table.insert(
            "trusted_hash".to_string(),
            TomlValue::String(hook.current_hash),
        );
        state_table.insert(hook.key, hook_state);
    }

    config_layer_stack
        .with_user_config(&codex_home.join(CONFIG_TOML_FILE), user_config)
        .expect("hook user config should be valid")
}
