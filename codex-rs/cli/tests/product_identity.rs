//! The product identity must hold under every name users actually invoke.
//!
//! Regression coverage for the split-brain bug where the `i` alias passed one
//! crate's brand check but not another's hand-rolled copy, so the process ran
//! with Open Interpreter branding on top of Codex's home directory, update
//! cache, and keychain credentials.

use std::path::Path;
use std::process::Command;

fn run(
    bin: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    removed: &[&str],
) -> anyhow::Result<(String, String)> {
    run_in(bin, args, envs, removed, /*cwd*/ None)
}

fn run_in(
    bin: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    removed: &[&str],
    cwd: Option<&Path>,
) -> anyhow::Result<(String, String)> {
    let mut command = Command::new(bin);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    for key in removed {
        command.env_remove(key);
    }
    let output = command.output()?;
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn canonical_str(path: &Path) -> anyhow::Result<String> {
    let path = path.canonicalize()?.to_string_lossy().into_owned();
    #[cfg(windows)]
    if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
        return Ok(format!(r"\\{path}"));
    } else if let Some(path) = path.strip_prefix(r"\\?\") {
        return Ok(path.to_string());
    }
    Ok(path)
}

fn json_contains_string(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value.contains(needle),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_string(value, needle)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| json_contains_string(value, needle)),
        _ => false,
    }
}

fn interpreter_alias(codex_bin: &Path, alias_dir: &Path) -> anyhow::Result<std::path::PathBuf> {
    let alias = alias_dir.join("interpreter");
    #[cfg(unix)]
    std::os::unix::fs::symlink(codex_bin, &alias)?;
    #[cfg(not(unix))]
    std::fs::copy(codex_bin, &alias)?;
    Ok(alias)
}

fn assert_no_codex_brand(surface: &str, stdout: &str, stderr: &str) {
    let output = format!("{stdout}\n{stderr}");
    assert!(
        !output.to_ascii_lowercase().contains("codex"),
        "{surface} leaked Codex branding:\n{output}"
    );
}

#[test]
fn interpreter_help_version_errors_and_completions_keep_product_identity() -> anyhow::Result<()> {
    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let alias_dir = tempfile::tempdir()?;
    let interpreter = interpreter_alias(&codex_bin, alias_dir.path())?;
    let interpreter_home = tempfile::tempdir()?;
    let home = canonical_str(interpreter_home.path())?;
    let envs = [("INTERPRETER_HOME", home.as_str())];

    for (surface, args) in [
        ("root help", &["--help"][..]),
        ("exec help", &["exec", "--help"][..]),
        ("completion help", &["completion", "--help"][..]),
        ("version", &["--version"][..]),
        (
            "invalid argument",
            &["--definitely-not-an-interpreter-option"][..],
        ),
        ("debug update error", &["update"][..]),
    ] {
        let (stdout, stderr) = run(&interpreter, args, &envs, &[])?;
        assert_no_codex_brand(surface, &stdout, &stderr);
    }

    for shell in ["bash", "zsh", "fish", "powershell"] {
        let (stdout, stderr) = run(&interpreter, &["completion", shell], &envs, &[])?;
        assert_no_codex_brand(shell, &stdout, &stderr);
        assert!(
            stdout.to_ascii_lowercase().contains("interpreter"),
            "{shell} completion should target interpreter: {stdout}"
        );
        assert!(
            stdout.contains("--chat-completions"),
            "{shell} completion should include --chat-completions: {stdout}"
        );
    }
    Ok(())
}

#[test]
fn i_alias_runs_as_open_interpreter_with_interpreter_home() -> anyhow::Result<()> {
    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let alias_dir = tempfile::tempdir()?;
    let alias = alias_dir.path().join("i");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&codex_bin, &alias)?;
    #[cfg(not(unix))]
    std::fs::copy(&codex_bin, &alias)?;

    let interpreter_home = tempfile::tempdir()?;
    let codex_home_decoy = tempfile::tempdir()?;
    let envs = [
        ("INTERPRETER_HOME", canonical_str(interpreter_home.path())?),
        ("CODEX_HOME", canonical_str(codex_home_decoy.path())?),
    ];
    let env_refs: Vec<(&str, &str)> = envs
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();

    // Brand: the version line must identify as interpreter, not codex.
    let (stdout, _) = run(&alias, &["--version"], &env_refs, &[])?;
    assert!(
        stdout.starts_with("interpreter "),
        "expected interpreter version line, got: {stdout}"
    );

    // Home isolation: the resolved home must be INTERPRETER_HOME, and the
    // CODEX_HOME decoy must not leak into the resolved configuration.
    let (stdout, stderr) = run(&alias, &["doctor", "--json"], &env_refs, &[])?;
    let interpreter_home_str = canonical_str(interpreter_home.path())?;
    let decoy_str = canonical_str(codex_home_decoy.path())?;
    let doctor: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(
        json_contains_string(&doctor, &interpreter_home_str),
        "doctor output should reference INTERPRETER_HOME {interpreter_home_str}; stdout: {stdout}; stderr: {stderr}"
    );
    assert!(
        !json_contains_string(&doctor, &decoy_str),
        "doctor output must not reference CODEX_HOME decoy {decoy_str}; stdout: {stdout}"
    );
    Ok(())
}

#[test]
fn codex_name_keeps_codex_identity_in_dev_builds() -> anyhow::Result<()> {
    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let (stdout, _) = run(&codex_bin, &["--version"], &[], &["OPEN_INTERPRETER_BRAND"])?;
    assert!(
        stdout.starts_with("codex "),
        "dev codex binary should keep codex identity, got: {stdout}"
    );
    Ok(())
}

#[test]
fn i_alias_never_loads_codex_project_config() -> anyhow::Result<()> {
    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let alias_dir = tempfile::tempdir()?;
    let alias = alias_dir.path().join("i");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&codex_bin, &alias)?;
    #[cfg(not(unix))]
    std::fs::copy(&codex_bin, &alias)?;

    let project = tempfile::tempdir()?;
    let project_path = canonical_str(project.path())?;
    let interpreter_home = tempfile::tempdir()?;
    let config = toml::Table::from_iter([(
        "projects".to_string(),
        toml::Value::Table(toml::Table::from_iter([(
            project_path,
            toml::Value::Table(toml::Table::from_iter([(
                "trust_level".to_string(),
                toml::Value::String("trusted".to_string()),
            )])),
        )])),
    )]);
    std::fs::write(
        interpreter_home.path().join("config.toml"),
        toml::to_string(&config)?,
    )?;

    // A repository's Codex configuration must never load as Interpreter
    // project config.
    std::fs::create_dir_all(project.path().join(".codex"))?;
    std::fs::write(
        project.path().join(".codex/config.toml"),
        "model = \"codex-leak-canary\"\n",
    )?;
    let home_str = canonical_str(interpreter_home.path())?;
    let envs = [("INTERPRETER_HOME", home_str.as_str())];
    let (stdout, stderr) = run_in(
        &alias,
        &["doctor", "--json"],
        &envs,
        &[],
        Some(project.path()),
    )?;
    assert!(
        !stdout.contains("codex-leak-canary") && !stderr.contains("codex-leak-canary"),
        "Interpreter must not load .codex project config; stdout: {stdout}"
    );

    // Interpreter's own project config folder does load.
    std::fs::create_dir_all(project.path().join(".openinterpreter"))?;
    std::fs::write(
        project.path().join(".openinterpreter/config.toml"),
        "model = \"interpreter-canary\"\n",
    )?;
    let (stdout, stderr) = run_in(
        &alias,
        &["doctor", "--json"],
        &envs,
        &[],
        Some(project.path()),
    )?;
    assert!(
        stdout.contains("interpreter-canary"),
        "Interpreter should load .openinterpreter project config; stdout: {stdout}; stderr: {stderr}"
    );
    Ok(())
}
