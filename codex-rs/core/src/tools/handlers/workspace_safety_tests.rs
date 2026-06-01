//! Regression tests for emulated-harness tool safety.
//!
//! Two guarantees that a single tool call must never be able to break,
//! regardless of what the model asks for:
//!
//! 1. **Containment** — every file read/write/edit/list/search tool honors the
//!    session filesystem policy, so the model cannot reach files outside the
//!    workspace when the policy forbids it.
//! 2. **Bounded traversal** — search tools that walk directories stay bounded
//!    and never follow symlink cycles, so one call cannot exhaust memory or
//!    hang the daemon.
//!
//! The tests drive the real tool handlers directly with a constructed
//! invocation under a workspace-only policy, so an out-of-workspace path or a
//! symlink cycle is exercised deterministically without a live model.

use super::ClaudeEditHandler;
use super::ClaudeGlobHandler;
use super::ClaudeGrepHandler;
use super::ClaudeReadHandler;
use super::ClaudeWriteHandler;
use super::DeepSeekTuiEditFileHandler;
use super::DeepSeekTuiFileSearchHandler;
use super::DeepSeekTuiGrepFilesHandler;
use super::DeepSeekTuiListDirHandler;
use super::DeepSeekTuiReadFileHandler;
use super::DeepSeekTuiWriteFileHandler;
use super::KimiGlobHandler;
use super::KimiGrepHandler;
use super::KimiReadFileHandler;
use super::KimiStrReplaceFileHandler;
use super::KimiWriteFileHandler;
use super::MinimalStrReplaceEditorHandler;
use super::OpenCodeEditHandler;
use super::OpenCodeGlobHandler;
use super::OpenCodeGrepHandler;
use super::OpenCodeReadHandler;
use super::OpenCodeWriteHandler;
use super::PiEditHandler;
use super::PiReadHandler;
use super::PiWriteHandler;
use super::QwenEditHandler;
use super::QwenGlobHandler;
use super::QwenGrepSearchHandler;
use super::QwenReadFileHandler;
use super::QwenWriteFileHandler;
use super::SweAgentCommandHandler;
use crate::session::session::Session;
use crate::session::tests::make_session_and_context;
use crate::session::turn_context::TurnContext;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolCallSource;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::registry::ToolHandler;
use crate::turn_diff_tracker::TurnDiffTracker;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use serde_json::json;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

fn invocation(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    tool_name: &str,
    arguments: Value,
) -> ToolInvocation {
    ToolInvocation {
        session,
        turn,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain(tool_name),
        source: ToolCallSource::Direct,
        payload: ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    }
}

/// Like [`invocation`], but for tools (e.g. SWE-agent) that take a custom
/// command-string payload instead of JSON function arguments.
fn custom_invocation(
    session: Arc<Session>,
    turn: Arc<TurnContext>,
    input: String,
) -> ToolInvocation {
    ToolInvocation {
        session,
        turn,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        tracker: Arc::new(Mutex::new(TurnDiffTracker::default())),
        call_id: "call_1".to_string(),
        tool_name: codex_tools::ToolName::plain("str_replace_editor"),
        source: ToolCallSource::Direct,
        payload: ToolPayload::Custom { input },
    }
}

/// Build a session/turn rooted at `workspace`, with a filesystem policy that
/// grants read+write inside `workspace` and denies everything else.
async fn workspace_only_session(workspace: &Path) -> (Arc<Session>, Arc<TurnContext>) {
    let (session, mut turn) = make_session_and_context().await;
    let workspace = AbsolutePathBuf::try_from(workspace.to_path_buf()).expect("absolute workspace");
    turn.cwd = workspace.clone();
    turn.file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: workspace },
            access: FileSystemAccessMode::Write,
        }]);
    (Arc::new(session), Arc::new(turn))
}

/// Assert that a JSON-payload tool refuses to touch a path outside the
/// workspace. `build_args` receives the out-of-workspace directory and a target
/// file inside it; `pre_create_target` seeds that file first so read/edit tools
/// would otherwise succeed (proving the refusal is the policy, not a missing
/// file). Write/create tools pass `false` and we additionally assert the file
/// was never created.
async fn assert_denied_outside<H>(
    handler: H,
    tool_name: &str,
    pre_create_target: bool,
    build_args: impl FnOnce(&Path, &Path) -> Value,
) where
    H: ToolHandler<Output = FunctionToolOutput>,
{
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let target = outside.path().join("evil.txt");
    if pre_create_target {
        std::fs::write(&target, b"existing-secret\n").expect("seed outside file");
    }
    let (session, turn) = workspace_only_session(workspace.path()).await;
    let args = build_args(outside.path(), &target);
    let result = handler
        .handle(invocation(session, turn, tool_name, args))
        .await;
    assert!(
        result.is_err(),
        "{tool_name}: out-of-workspace access must be denied by the session policy"
    );
    if !pre_create_target {
        assert!(
            !target.exists(),
            "{tool_name}: a denied write must not create the file"
        );
    }
}

/// Assert that a directory-walking search tool terminates (does not run away)
/// when the workspace contains a symlink cycle, and still returns successfully.
#[cfg(unix)]
async fn assert_search_terminates<H>(
    handler: H,
    tool_name: &str,
    build_args: impl FnOnce(&Path) -> Value,
) where
    H: ToolHandler<Output = FunctionToolOutput>,
{
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    std::fs::create_dir(workspace.path().join("sub")).expect("create sub");
    std::fs::write(workspace.path().join("sub/target.txt"), b"needle").expect("write file");
    // A symlink back to the workspace root recurses forever if followed.
    symlink(workspace.path(), workspace.path().join("sub/loop")).expect("symlink loop");
    let (session, turn) = workspace_only_session(workspace.path()).await;
    let args = build_args(workspace.path());
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        handler.handle(invocation(session, turn, tool_name, args)),
    )
    .await;
    assert!(
        result.is_ok(),
        "{tool_name}: search did not terminate on a symlink cycle (runaway)"
    );
    assert!(
        result.unwrap().is_ok(),
        "{tool_name}: search over a valid workspace returned an error"
    );
}

// ----- Containment: writes/creates (denied outside, must not create) -----

#[tokio::test]
async fn claude_write_denied_outside() {
    assert_denied_outside(
        ClaudeWriteHandler,
        "Write",
        false,
        |_d, f| json!({ "file_path": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn kimi_write_denied_outside() {
    assert_denied_outside(
        KimiWriteFileHandler,
        "write_file",
        false,
        |_d, f| json!({ "path": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn deepseek_write_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiWriteFileHandler,
        "write_file",
        false,
        |_d, f| json!({ "path": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn opencode_write_denied_outside() {
    assert_denied_outside(
        OpenCodeWriteHandler,
        "write",
        false,
        |_d, f| json!({ "filePath": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn pi_write_denied_outside() {
    assert_denied_outside(
        PiWriteHandler,
        "write",
        false,
        |_d, f| json!({ "path": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn qwen_write_denied_outside() {
    assert_denied_outside(
        QwenWriteFileHandler,
        "write_file",
        false,
        |_d, f| json!({ "path": f.to_string_lossy(), "content": "owned" }),
    )
    .await;
}

// ----- Containment: edits (denied outside; file pre-seeded) -----

#[tokio::test]
async fn claude_edit_denied_outside() {
    assert_denied_outside(ClaudeEditHandler, "Edit", true, |_d, f| {
        json!({ "file_path": f.to_string_lossy(), "old_string": "existing-secret", "new_string": "x" })
    })
    .await;
}

#[tokio::test]
async fn kimi_str_replace_denied_outside() {
    assert_denied_outside(KimiStrReplaceFileHandler, "str_replace", true, |_d, f| {
        json!({ "path": f.to_string_lossy(), "old_string": "existing-secret", "new_string": "x" })
    })
    .await;
}

#[tokio::test]
async fn deepseek_edit_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiEditFileHandler,
        "edit_file",
        true,
        |_d, f| json!({ "path": f.to_string_lossy(), "search": "existing-secret", "replace": "x" }),
    )
    .await;
}

#[tokio::test]
async fn opencode_edit_denied_outside() {
    assert_denied_outside(OpenCodeEditHandler, "edit", true, |_d, f| {
        json!({ "filePath": f.to_string_lossy(), "oldString": "existing-secret", "newString": "x" })
    })
    .await;
}

#[tokio::test]
async fn pi_edit_denied_outside() {
    assert_denied_outside(PiEditHandler, "edit", true, |_d, f| {
        json!({ "path": f.to_string_lossy(), "edits": [{ "oldText": "existing-secret", "newText": "x" }] })
    })
    .await;
}

#[tokio::test]
async fn qwen_edit_denied_outside() {
    assert_denied_outside(QwenEditHandler, "edit", true, |_d, f| {
        json!({ "file_path": f.to_string_lossy(), "old_string": "existing-secret", "new_string": "x" })
    })
    .await;
}

// ----- Containment: reads (denied outside; file pre-seeded) -----

#[tokio::test]
async fn claude_read_denied_outside() {
    assert_denied_outside(
        ClaudeReadHandler,
        "Read",
        true,
        |_d, f| json!({ "file_path": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn kimi_read_denied_outside() {
    assert_denied_outside(
        KimiReadFileHandler,
        "read_file",
        true,
        |_d, f| json!({ "path": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn deepseek_read_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiReadFileHandler,
        "read_file",
        true,
        |_d, f| json!({ "path": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn opencode_read_denied_outside() {
    assert_denied_outside(
        OpenCodeReadHandler,
        "read",
        true,
        |_d, f| json!({ "filePath": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn pi_read_denied_outside() {
    assert_denied_outside(
        PiReadHandler,
        "read",
        true,
        |_d, f| json!({ "path": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn qwen_read_denied_outside() {
    assert_denied_outside(
        QwenReadFileHandler,
        "read_file",
        true,
        |_d, f| json!({ "file_path": f.to_string_lossy() }),
    )
    .await;
}

// ----- Containment: list / search / glob / grep over an outside dir -----

#[tokio::test]
async fn deepseek_list_dir_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiListDirHandler,
        "list_dir",
        true,
        |d, _f| json!({ "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn deepseek_file_search_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiFileSearchHandler,
        "file_search",
        true,
        |d, _f| json!({ "query": "evil", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn deepseek_grep_files_denied_outside() {
    assert_denied_outside(
        DeepSeekTuiGrepFilesHandler,
        "grep_files",
        true,
        |d, _f| json!({ "pattern": "secret", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn opencode_grep_denied_outside() {
    assert_denied_outside(
        OpenCodeGrepHandler,
        "grep",
        true,
        |d, _f| json!({ "pattern": "secret", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn opencode_glob_denied_outside() {
    assert_denied_outside(
        OpenCodeGlobHandler,
        "glob",
        true,
        |d, _f| json!({ "pattern": "*.txt", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn claude_glob_denied_outside() {
    assert_denied_outside(
        ClaudeGlobHandler,
        "Glob",
        true,
        |d, _f| json!({ "pattern": "*.txt", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn claude_grep_denied_outside() {
    assert_denied_outside(
        ClaudeGrepHandler,
        "Grep",
        true,
        |d, _f| json!({ "pattern": "secret", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn kimi_glob_denied_outside() {
    assert_denied_outside(
        KimiGlobHandler,
        "glob",
        true,
        |d, _f| json!({ "pattern": "*.txt", "directory": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn kimi_grep_denied_outside() {
    assert_denied_outside(
        KimiGrepHandler,
        "grep",
        true,
        |d, _f| json!({ "pattern": "secret", "path": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn qwen_glob_denied_outside() {
    assert_denied_outside(
        QwenGlobHandler,
        "glob",
        true,
        |d, _f| json!({ "pattern": "*.txt", "directory": d.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn qwen_grep_search_denied_outside() {
    assert_denied_outside(
        QwenGrepSearchHandler,
        "grep",
        true,
        |d, _f| json!({ "pattern": "secret", "path": d.to_string_lossy() }),
    )
    .await;
}

// ----- Containment: minimal editor (command-tagged JSON) -----

#[tokio::test]
async fn minimal_create_denied_outside() {
    assert_denied_outside(
        MinimalStrReplaceEditorHandler,
        "str_replace_editor",
        false,
        |_d, f| json!({ "command": "create", "path": f.to_string_lossy(), "file_text": "owned" }),
    )
    .await;
}

#[tokio::test]
async fn minimal_view_denied_outside() {
    assert_denied_outside(
        MinimalStrReplaceEditorHandler,
        "str_replace_editor",
        true,
        |_d, f| json!({ "command": "view", "path": f.to_string_lossy() }),
    )
    .await;
}

#[tokio::test]
async fn minimal_str_replace_denied_outside() {
    assert_denied_outside(MinimalStrReplaceEditorHandler, "str_replace_editor", true, |_d, f| {
        json!({ "command": "str_replace", "path": f.to_string_lossy(), "old_str": "existing-secret", "new_str": "x" })
    })
    .await;
}

#[tokio::test]
async fn minimal_insert_denied_outside() {
    assert_denied_outside(MinimalStrReplaceEditorHandler, "str_replace_editor", true, |_d, f| {
        json!({ "command": "insert", "path": f.to_string_lossy(), "insert_line": 1, "new_str": "x" })
    })
    .await;
}

// ----- Containment: SWE-agent editor (custom command string) -----

async fn assert_swe_agent_denied_outside(command: impl FnOnce(&Path) -> String, pre_create: bool) {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let target = outside.path().join("evil.txt");
    if pre_create {
        std::fs::write(&target, b"existing-secret\n").expect("seed outside file");
    }
    let (session, turn) = workspace_only_session(workspace.path()).await;
    let input = command(&target);
    let result = SweAgentCommandHandler
        .handle(custom_invocation(session, turn, input))
        .await;
    assert!(
        result.is_err(),
        "swe-agent: out-of-workspace access must be denied"
    );
    if !pre_create {
        assert!(
            !target.exists(),
            "swe-agent: denied create must not create the file"
        );
    }
}

#[tokio::test]
async fn swe_agent_create_denied_outside() {
    assert_swe_agent_denied_outside(
        |f| {
            format!(
                "str_replace_editor create {} --file_text owned",
                f.to_string_lossy()
            )
        },
        false,
    )
    .await;
}

#[tokio::test]
async fn swe_agent_view_denied_outside() {
    assert_swe_agent_denied_outside(
        |f| format!("str_replace_editor view {}", f.to_string_lossy()),
        true,
    )
    .await;
}

#[tokio::test]
async fn swe_agent_str_replace_denied_outside() {
    assert_swe_agent_denied_outside(
        |f| {
            format!(
                "str_replace_editor str_replace {} --old_str existing-secret --new_str x",
                f.to_string_lossy()
            )
        },
        true,
    )
    .await;
}

// ----- Containment: positive control (in-workspace write allowed) -----

#[tokio::test]
async fn deepseek_write_allowed_inside_workspace() {
    let workspace = tempfile::tempdir().expect("workspace tempdir");
    let (session, turn) = workspace_only_session(workspace.path()).await;
    let inside = workspace.path().join("ok.txt");

    DeepSeekTuiWriteFileHandler
        .handle(invocation(
            session,
            turn,
            "write_file",
            json!({ "path": inside.to_string_lossy(), "content": "fine" }),
        ))
        .await
        .expect("writing inside the workspace must be allowed");
    assert!(inside.exists(), "in-workspace write should succeed");
}

// ----- Bounded traversal: tools using the shared safe walker -----

#[cfg(unix)]
#[tokio::test]
async fn deepseek_file_search_is_bounded_on_symlink_cycle() {
    assert_search_terminates(
        DeepSeekTuiFileSearchHandler,
        "file_search",
        |_w| json!({ "query": "target" }),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn deepseek_grep_files_is_bounded_on_symlink_cycle() {
    assert_search_terminates(
        DeepSeekTuiGrepFilesHandler,
        "grep_files",
        |_w| json!({ "pattern": "needle" }),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn opencode_grep_is_bounded_on_symlink_cycle() {
    assert_search_terminates(
        OpenCodeGrepHandler,
        "grep",
        |_w| json!({ "pattern": "needle" }),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn opencode_glob_is_bounded_on_symlink_cycle() {
    assert_search_terminates(
        OpenCodeGlobHandler,
        "glob",
        |_w| json!({ "pattern": "**/*.txt" }),
    )
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn kimi_glob_is_bounded_on_symlink_cycle() {
    // A mid-pattern `**` (not a `**` prefix, which kimi rejects up front) still
    // recurses, so it would loop on the symlink cycle without the bounded walk.
    assert_search_terminates(KimiGlobHandler, "glob", |_w| json!({ "pattern": "sub/**" })).await;
}

#[cfg(unix)]
#[tokio::test]
async fn qwen_glob_is_bounded_on_symlink_cycle() {
    assert_search_terminates(QwenGlobHandler, "glob", |_w| json!({ "pattern": "sub/**" })).await;
}
