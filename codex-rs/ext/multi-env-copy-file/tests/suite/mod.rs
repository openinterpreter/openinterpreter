//! Covers opt-in file copying, argument validation, and independent endpoint permissions.

use anyhow::Context;
use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_exec_server::ReadFileOptions;
use codex_extension_api::ExtensionRegistryBuilder;
use codex_features::Feature;
use codex_protocol::protocol::EnvironmentConfigState;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use codex_utils_path_uri::PathConvention;
use codex_utils_path_uri::PathUri;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_function_call_with_namespace;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::environment_config_for_selection;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;
use test_case::test_case;

mod connectivity;
mod transfers;

/// Host registration controls visibility, even before a turn selects any environments.
#[test_case(false, true; "disabled_by_default")]
#[test_case(true, false; "no_environments")]
#[test_case(true, true; "one_environment")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_file_is_opt_in(enabled: bool, attach_environment: bool) -> Result<()> {
    let server = start_mock_server().await;
    let mut extensions = ExtensionRegistryBuilder::new();
    if enabled {
        codex_multi_env_copy_file::install(&mut extensions);
    }
    let test = test_codex()
        .with_extensions(Arc::new(extensions.build()))
        .build_with_auto_env(&server)
        .await?;
    let selections = attach_environment
        .then(|| test.executor_environment().selection().clone())
        .into_iter()
        .collect();
    let mock = core_test_support::responses::mount_sse_once(
        &server,
        sse(vec![
            ev_assistant_message("done", "done"),
            ev_completed("done-response"),
        ]),
    )
    .await;
    test.submit_turn_with_environments("List available tools.", Some(selections))
        .await?;
    let request = mock.single_request();
    let body = request.body_json();
    assert_eq!(
        body["tools"]
            .as_array()
            .context("tools")?
            .iter()
            .any(|tool| tool["name"] == "multi_env_copy_file")
            || request
                .tool_by_name("functions", "multi_env_copy_file")
                .is_some(),
        enabled
    );
    Ok(())
}

/// Invalid endpoints fail before truncating a target; directories and missing parents are rejected.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_file_validates_endpoints() -> Result<()> {
    use codex_protocol::protocol::TurnEnvironmentSelection;
    use core_test_support::exec_server::ExecServerProcess;
    let executor = ExecServerProcess::start().await?;
    let root = TempDir::new()?;
    let server = start_mock_server().await;
    let mut extensions = ExtensionRegistryBuilder::new();
    codex_multi_env_copy_file::install(&mut extensions);
    let test = test_codex()
        .with_extensions(Arc::new(extensions.build()))
        .build_with_auto_env(&server)
        .await?;
    let primary_file = test.workspace_path_uri("primary.txt")?;
    test.fs()
        .write_file(
            &primary_file,
            b"primary".to_vec(),
            Default::default(),
            /*sandbox*/ None,
        )
        .await?;
    let second_file = root.path().join("second.txt");
    fs::write(&second_file, b"second")?;
    let selection = test.executor_environment().selection();
    let primary_path = primary_file.inferred_native_path_string();
    let primary = (selection.environment_id.as_str(), primary_path.as_str());
    let second = ("second", second_file.to_str().context("second file path")?);
    let convention = selection
        .cwd
        .infer_path_convention()
        .context("cwd path convention")?;
    let foreign_path = match convention {
        PathConvention::Posix => r"Z:\foreign\source.txt",
        PathConvention::Windows => "/foreign/source.txt",
    };
    let cases = [
        ("source_environment_id", None, "missing field"),
        ("target_environment_id", None, "missing field"),
        (
            "source_environment_id",
            Some("unknown".to_string()),
            "unknown",
        ),
        (
            "target_environment_id",
            Some("unknown".to_string()),
            "unknown",
        ),
        ("source_path", Some("source.txt".to_string()), "absolute"),
        ("target_path", Some("target.txt".to_string()), "absolute"),
        ("source_path", Some(foreign_path.to_string()), "absolute"),
        (
            "source_path",
            Some(
                test.workspace_path_uri("missing.bin")?
                    .inferred_native_path_string(),
            ),
            "could not open copy source",
        ),
        (
            "target_path",
            Some(selection.cwd.inferred_native_path_string()),
            "copy failed",
        ),
        (
            "source_path",
            Some(selection.cwd.inferred_native_path_string()),
            "could not open copy source",
        ),
        (
            "target_path",
            Some(
                test.workspace_path_uri("missing/target.txt")?
                    .inferred_native_path_string(),
            ),
            "target may be truncated or partially written",
        ),
    ];
    let mut calls = Vec::new();
    for (index, (field, value, _)) in cases.iter().enumerate() {
        // Exercise the selected executor's path conventions for both source and target errors.
        let ((source_id, source), (target_id, target)) = if field.starts_with("source_") {
            (primary, second)
        } else {
            (second, primary)
        };
        let mut args = json!({
            "source_environment_id": source_id,
            "source_path": source,
            "target_environment_id": target_id,
            "target_path": target,
        });
        if let Some(value) = value {
            args[field] = json!(value);
        } else {
            args.as_object_mut().unwrap().remove(*field);
        }
        calls.push(ev_function_call_with_namespace(
            &format!("invalid-{index}"),
            "functions",
            "multi_env_copy_file",
            &args.to_string(),
        ));
    }
    calls.push(ev_completed("invalid-copies"));
    let mock = mount_sse_sequence(
        &server,
        vec![
            sse(calls),
            sse(vec![
                ev_assistant_message("done", "done"),
                ev_completed("done"),
            ]),
        ],
    )
    .await;
    let manager = test.thread_manager.environment_manager();
    manager.upsert_environment(
        "second".to_string(),
        executor.websocket_url.clone(),
        /*connect_timeout*/ None,
    )?;
    manager
        .get_environment("second")
        .context("second environment")?
        .wait_until_ready()
        .await?;
    let cwd = PathUri::from_host_native_path(root.path())?;
    test.submit_turn_with_environments(
        "Try copying with these invalid endpoints.",
        Some(vec![
            selection.clone(),
            TurnEnvironmentSelection {
                selected_capability_roots: Default::default(),
                environment_id: "second".to_string(),
                cwd: cwd.clone(),
                workspace_roots: vec![cwd],
                config: EnvironmentConfigState::FromThread,
            },
        ]),
    )
    .await?;
    let mut errors = Vec::new();
    for (index, (field, value, expected)) in cases.iter().enumerate() {
        let output = mock
            .function_call_output_text(&format!("invalid-{index}"))
            .context("copy error")?;
        assert!(output.contains(expected), "{index}: {output}");
        let normalize = |text: &str| {
            text.replace(&selection.cwd.to_string(), "<CWD_URI>")
                .replace(&selection.cwd.inferred_native_path_string(), "<CWD>")
                .replace(
                    &format!("for environment {}", selection.environment_id),
                    "for environment <PRIMARY>",
                )
        };
        errors.push(json!({
            "field": field,
            "value": value.as_deref().map(normalize),
            "output": normalize(&output),
        }));
    }
    // JSON error offsets depend on the workspace path's length.
    let mut filters = vec![(r"at line 1 column \d+", "at line 1 column <COLUMN>")];
    let platform = match convention {
        PathConvention::Posix => "unix",
        PathConvention::Windows => {
            // Opening a directory as a file reports PermissionDenied on native Windows
            // and InvalidInput through the Wine executor.
            filters.push((
                r": (?:permission denied|invalid input parameter)",
                ": <DIRECTORY_ERROR>",
            ));
            "windows"
        }
    };
    insta::with_settings!({snapshot_suffix => platform, sort_maps => true, filters => filters}, {
        insta::assert_json_snapshot!("endpoint_errors", errors);
    });
    assert_eq!(
        test.fs()
            .read_file(
                &primary_file,
                ReadFileOptions::default(),
                /*sandbox*/ None
            )
            .await?,
        b"primary"
    );
    assert_eq!(fs::read(second_file)?, b"second");
    Ok(())
}

/// Each endpoint uses its own executor and permissions, even for allowed paths outside its cwd.
#[test_case(false; "environment_profiles")]
#[test_case(true; "turn_and_session_grants")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn copy_file_uses_each_environments_permissions(use_grants: bool) -> Result<()> {
    use codex_protocol::models::PermissionProfile;
    use codex_protocol::models::PermissionProfileSnapshot;
    use codex_protocol::permissions::FileSystemAccessMode;
    use codex_protocol::permissions::FileSystemSandboxEntry;
    use codex_protocol::permissions::FileSystemSandboxPolicy;
    use codex_protocol::permissions::NetworkSandboxPolicy;
    use codex_protocol::protocol::AskForApproval;
    use codex_protocol::protocol::ThreadSettingsOverrides;
    use codex_protocol::protocol::TurnEnvironmentSelection;
    use codex_protocol::protocol::TurnEnvironmentSelections;
    use codex_protocol::request_permissions::PermissionGrantScope;
    use codex_protocol::request_permissions::RequestPermissionsResponse;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use core_test_support::exec_server::ExecServerProcess;

    let source_executor = ExecServerProcess::start().await?;
    let target_executor = ExecServerProcess::start().await?;
    let source_root = TempDir::new()?;
    let target_root = TempDir::new()?;
    let source = source_root.path().join("source.bin");
    let target = target_root.path().join("target.bin");
    let denied_source = target_root.path().join("private.bin");
    let denied_target = source_root.path().join("copy.bin");
    let bytes = [0, 255, 17, 42].repeat(/*n*/ 300_000);
    fs::write(&source, &bytes)?;
    fs::write(&target, b"original")?;
    fs::write(&denied_source, b"private")?;
    let server = start_mock_server().await;
    let mut extensions = ExtensionRegistryBuilder::new();
    codex_multi_env_copy_file::install(&mut extensions);
    let test = test_codex()
        .with_extensions(Arc::new(extensions.build()))
        .with_config(|config| {
            // Make the copy itself the first restricted read. On unelevated Windows the
            // executor rejects this policy, which would otherwise stop AGENTS.md discovery.
            config.project_doc_max_bytes = 0;
            config
                .features
                .enable(Feature::RequestPermissionsTool)
                .expect("enable request_permissions");
        })
        .build_with_auto_env(&server)
        .await?;
    let mut selections = Vec::new();
    for (id, executor, root, access) in [
        (
            "source",
            &source_executor,
            &source_root,
            FileSystemAccessMode::Read,
        ),
        (
            "target",
            &target_executor,
            &target_root,
            FileSystemAccessMode::Write,
        ),
    ] {
        fs::create_dir(root.path().join("cwd"))?;
        let manager = test.thread_manager.environment_manager();
        manager.upsert_environment(
            id.to_string(),
            executor.websocket_url.clone(),
            /*connect_timeout*/ None,
        )?;
        manager
            .get_environment(id)
            .context("environment")?
            .wait_until_ready()
            .await?;
        let cwd = PathUri::from_host_native_path(root.path().join("cwd"))?;
        let mut selection = TurnEnvironmentSelection {
            selected_capability_roots: Default::default(),
            environment_id: id.to_string(),
            cwd: cwd.clone(),
            workspace_roots: vec![cwd],
            config: EnvironmentConfigState::FromThread,
        };
        let mut config = environment_config_for_selection(&test.config, &selection);
        config.permission_profile =
            PermissionProfileSnapshot::legacy(PermissionProfile::from_runtime_permissions(
                &FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry::new(
                    AbsolutePathBuf::from_absolute_path(if use_grants {
                        root.path().join("cwd")
                    } else {
                        root.path().to_path_buf()
                    })?
                    .into(),
                    if use_grants {
                        FileSystemAccessMode::Read
                    } else {
                        access
                    },
                )]),
                NetworkSandboxPolicy::Restricted,
            ));
        selection.config = EnvironmentConfigState::Ready(config);
        selections.push(selection);
    }
    let mut calls = Vec::new();
    for (id, source_path, target_path) in [
        ("copy", &source, &target),
        ("denied-read", &denied_source, &target),
        ("denied-write", &source, &denied_target),
    ] {
        calls.push(ev_function_call_with_namespace(
            id,
            "functions",
            "multi_env_copy_file",
            &json!({
                "source_environment_id": "source", "source_path": source_path,
                "target_environment_id": "target", "target_path": target_path,
            })
            .to_string(),
        ));
    }
    calls.push(ev_completed("copies"));
    let mut responses = Vec::new();
    if use_grants {
        let mut requests = Vec::new();
        for (id, root, access) in [
            ("source", &source_root, "read"),
            ("target", &target_root, "write"),
        ] {
            requests.push(ev_function_call_with_namespace(
                id,
                "functions",
                "request_permissions",
                &json!({
                    "environment_id": id,
                    "reason": "Copy an artifact outside the working directory",
                    "permissions": {"file_system": {access: [root.path()]}},
                })
                .to_string(),
            ));
        }
        requests.push(ev_completed("permissions"));
        responses.push(sse(requests));
    }
    responses.extend([
        sse(calls),
        sse(vec![
            ev_assistant_message("done", "done"),
            ev_completed("done"),
        ]),
    ]);
    let mock = mount_sse_sequence(&server, responses).await;
    test.codex
        .start_or_steer_turn(
            TurnInputRequest::user_input(vec![UserInput::Text {
                text: "Copy the artifact to the target environment.".to_string(),
                text_elements: Vec::new(),
            }])
            .with_thread_settings(ThreadSettingsOverrides {
                environments: Some(
                    TurnEnvironmentSelections::new(test.config.cwd.clone(), selections)
                        .into_requests(),
                ),
                approval_policy: Some(AskForApproval::OnRequest),
                ..Default::default()
            }),
        )
        .await?;
    if use_grants {
        for scope in [PermissionGrantScope::Turn, PermissionGrantScope::Session] {
            let event = wait_for_event(&test.codex, |event| {
                matches!(
                    event,
                    EventMsg::RequestPermissions(_) | EventMsg::TurnComplete(_)
                )
            })
            .await;
            let EventMsg::RequestPermissions(request) = event else {
                anyhow::bail!("copy finished before requesting permissions");
            };
            test.codex
                .submit(Op::RequestPermissionsResponse {
                    id: request.call_id,
                    response: RequestPermissionsResponse {
                        permissions: request.permissions,
                        scope,
                        strict_auto_review: false,
                    },
                })
                .await?;
        }
    }
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let copy = mock
        .function_call_output_text("copy")
        .context("copy result")?;
    let read_error = mock
        .function_call_output_text("denied-read")
        .context("read error")?;
    let write_error = mock
        .function_call_output_text("denied-write")
        .context("write error")?;
    if cfg!(windows) {
        // These two executors are host-native. The unelevated Windows sandbox must reject
        // a policy with restricted reads, including after granting access to a specific path.
        for error in [copy, read_error, write_error] {
            assert!(error.contains("could not open copy source"), "{error}");
        }
        assert_eq!(fs::read(&source)?, bytes);
        assert_eq!(fs::read(&target)?, b"original");
        assert!(!denied_target.try_exists()?);
        return Ok(());
    }
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&copy)?,
        json!({"bytes_copied": bytes.len()})
    );
    assert!(read_error.contains("could not open copy source"));
    assert!(write_error.contains("target may be truncated or partially written"));
    // Linux hides inaccessible roots; other sandbox implementations may report a denial instead.
    #[cfg(target_os = "linux")]
    if !use_grants {
        insta::assert_json_snapshot!("permission_errors", [read_error, write_error]);
    }
    assert_eq!(
        (fs::read(&source)?, fs::read(&target)?),
        (bytes.clone(), bytes)
    );
    assert!(!denied_target.try_exists()?);
    Ok(())
}
