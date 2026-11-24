use super::write_command;
use pretty_assertions::assert_eq;
use std::time::Duration;
use std::time::Instant;

#[tokio::test]
async fn installed_adapter_receives_compatible_command_and_never_copies_expired_work() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let executable = root.join("powershell.exe");
    std::fs::write(
        &executable,
        r#"#!/bin/sh
[ "$#" -eq 3 ] && [ "$1" = '-NoProfile' ] && [ "$2" = '-Command' ] || exit 46
[ "$3" = "[Console]::InputEncoding = [System.Text.Encoding]::UTF8; \$ErrorActionPreference = 'Stop'; \$text = [Console]::In.ReadToEnd(); Set-Clipboard -Value \$text" ] || exit 47
/bin/cat > "$COPY_OUTPUT"
"#,
    ).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = root.join("clipboard");
    let mut command = super::command(&executable, root.as_os_str());
    command.env("COPY_OUTPUT", &output);
    let text = " café 🦀\r\n\t".repeat(/*n*/ 32_768);
    let status = write_command(
        command,
        &text,
        Instant::now() + Duration::from_secs(/*secs*/ 5),
    )
    .await
    .unwrap();
    assert!(status.success(), "{status}");
    assert_eq!(std::fs::read_to_string(&output).unwrap(), text);
    let mut command = super::command(&executable, root.as_os_str());
    command.env("COPY_OUTPUT", &output);
    assert_eq!(
        write_command(command, "late", Instant::now()).await,
        Err("Windows clipboard writer timed out".into())
    );
    assert_eq!(std::fs::read_to_string(output).unwrap(), text);
}

#[tokio::test]
async fn stalled_input_and_stalled_exit_are_reaped_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    // Large input blocks on the pipe; small input fits, so the worker reaches the exit wait.
    for (stage, text, script) in [
        (
            "input",
            "x".repeat(/*n*/ 1_048_576),
            "echo $$ > \"$1\"; exec sleep 30",
        ),
        ("exit", "small".into(), "echo $$ > \"$1\"; exec sleep 30"),
        // An adapter waiting on a subprocess must stop that subprocess too.
        (
            "subprocess",
            String::new(),
            "sh -c 'echo $$ > \"$1\"; exec sleep 30' sh \"$1\"; wait",
        ),
    ] {
        let pid_path = dir.path().join(format!("{stage}.pid"));
        let mut command = tokio::process::Command::new("sh");
        command.args(["-c", script, "sh"]).arg(&pid_path);
        let started = Instant::now();
        assert_eq!(
            write_command(command, &text, started + Duration::from_secs(/*secs*/ 1)).await,
            Err("Windows clipboard writer timed out".into())
        );
        assert!(
            started.elapsed() < Duration::from_secs(/*secs*/ 10),
            "{stage}: cleanup exceeded its budget: {:?}",
            started.elapsed()
        );
        let pid: libc::pid_t = std::fs::read_to_string(&pid_path)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        if stage == "subprocess" {
            // The grandchild must be stopped; only init can reap it. A zombie is
            // acceptable here and still responds to kill(pid, 0) until reaped.
            let output = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()
                .unwrap();
            let state = String::from_utf8(output.stdout).unwrap();
            let state = state.trim();
            assert!(state.is_empty() || state.starts_with('Z'), "state: {state}");
        } else {
            // SAFETY: Signal 0 observes this disposable child without modifying it.
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
        }
    }
}
