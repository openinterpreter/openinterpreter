# Open Interpreter CLI

## Critical CLI Interface Contract

`interpreter` is the user-facing Open Interpreter command and must remain a
drop-in replacement for the Codex CLI surface. We are not redesigning upstream
Codex CLI usage under a different name: subcommands, flags, and non-interactive
flows should preserve the behavior users would expect from the corresponding
Codex command under the Open Interpreter name.

The intentional exception is bare interactive startup: running `interpreter`
without a subcommand starts Open Interpreter's app-server-backed interactive TUI.
On first interactive use, that path starts the local app-server daemon and then
connects the TUI through it.
`interpreter exec` and the standalone `interpreter-exec` binary both use the
same non-interactive exec implementation.

## Default Low-Memory TUI Mode

Open Interpreter keeps interactive TUI clients lightweight by default. Completed
transcript cells are written to terminal/deferred history instead of being kept
in every live TUI client's in-memory transcript list, and very large active tool
outputs are capped in the live client while preserving their tail with an
output-truncated marker. This changes client memory use; it does not change the
conversation state sent to the model.

Set `INTERPRETER_TUI_LOW_MEMORY=0` to disable this default. Advanced debugging
can override the two pieces independently with
`INTERPRETER_TUI_DROP_COMMITTED_HISTORY` and
`INTERPRETER_TUI_ACTIVE_EXEC_OUTPUT_MAX_BYTES`.
