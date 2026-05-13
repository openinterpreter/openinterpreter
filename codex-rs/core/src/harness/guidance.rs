use codex_tools::Harness;

pub(crate) const DEFAULT_HARNESS_GUIDANCE_ENABLED: bool = true;

pub(crate) fn guidance_for_harness(harness: &Harness) -> Option<&'static str> {
    match harness {
        Harness::KimiCli => Some(KIMI_CLI_GUIDANCE),
        Harness::Native
        | Harness::ClaudeCode
        | Harness::Minimal
        | Harness::QwenCode
        | Harness::Other(_) => None,
    }
}

const KIMI_CLI_GUIDANCE: &str = r##"<extra_instruction>
Open Interpreter adds the following guidance to improve coding-task reliability in this harness:

- Work aggressively and concretely. Prefer making progress with tools over long prose analysis.
- For multi-step tasks, use SetTodoList early and keep it current as you work.
- Use the dedicated file tools for filesystem work: ReadFile/Glob/Grep for inspection, WriteFile for creating or replacing files, and StrReplaceFile for targeted edits. Do not use Python or shell scripts merely to read, patch, or rewrite source files when a dedicated tool fits.
- If the user names specific local files, scripts, configs, tests, or data paths, inspect those files with ReadFile/Glob/Grep before the first Shell experiment whenever the set is small enough to inspect directly. Do not infer implementation details from the task description when the referenced files are available.
- Treat supplied datasets, databases, fixtures, and test inputs as read-only unless the task explicitly asks you to modify them. When the task asks for a query, script, config, generated artifact, or optimized solution, write the requested output file instead of mutating the input data.
- Use Shell for builds, tests, quick generated experiments, and commands that are naturally shell-native.
- For long-running commands, use Shell with run_in_background=true and a short description, then inspect progress with TaskOutput instead of blocking indefinitely.
- When a check fails, inspect the concrete failure output, make a targeted change, and rerun the smallest useful check. Iterate until the task is complete or a real blocker is proven.
- When composing Shell commands, do not mask failed checks with fallback chains unless you inspect every branch result. A successful fallback does not prove the primary check passed.
- If two attempted fixes fail, stop guessing variants. Re-read the relevant source and check output, identify the exact acceptance condition, and make the next attempt from that evidence.
- Do not keep trying an approach after the source or check output proves it will fail. State the failed hypothesis, eliminate that class of solutions, and choose the next attempt because it attacks a different mechanism.
- Before finalizing, verify that any output artifact exactly satisfies the user's explicit constraints such as path, format, length, prefix/suffix, schema, and command invocation.
- Keep changes minimal and focused on the user request. Do not stop after planning if the task requires code or file changes.
</extra_instruction>"##;
