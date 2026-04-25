use super::*;
use std::process::Command as StdCommand;
use tempfile::tempdir;

#[test]
fn parses_basic_results() {
    let stdout = b"/tmp/file_a.rs\n/tmp/file_b.rs\n";
    let parsed = parse_results(stdout, 10);
    assert_eq!(
        parsed,
        vec!["/tmp/file_a.rs".to_string(), "/tmp/file_b.rs".to_string()]
    );
}

#[test]
fn parse_truncates_after_limit() {
    let stdout = b"/tmp/file_a.rs\n/tmp/file_b.rs\n/tmp/file_c.rs\n";
    let parsed = parse_results(stdout, 2);
    assert_eq!(
        parsed,
        vec!["/tmp/file_a.rs".to_string(), "/tmp/file_b.rs".to_string()]
    );
}

#[tokio::test]
async fn run_search_returns_results() -> anyhow::Result<()> {
    if !rg_available() {
        return Ok(());
    }
    let temp = tempdir().expect("create temp dir");
    let dir = temp.path();
    std::fs::write(dir.join("match_one.txt"), "alpha beta gamma").unwrap();
    std::fs::write(dir.join("match_two.txt"), "alpha delta").unwrap();
    std::fs::write(dir.join("other.txt"), "omega").unwrap();

    let results = run_rg_search("alpha", None, dir, 10, dir).await?;
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|path| path.ends_with("match_one.txt")));
    assert!(results.iter().any(|path| path.ends_with("match_two.txt")));
    Ok(())
}

#[tokio::test]
async fn run_search_with_glob_filter() -> anyhow::Result<()> {
    if !rg_available() {
        return Ok(());
    }
    let temp = tempdir().expect("create temp dir");
    let dir = temp.path();
    std::fs::write(dir.join("match_one.rs"), "alpha beta gamma").unwrap();
    std::fs::write(dir.join("match_two.txt"), "alpha delta").unwrap();

    let results = run_rg_search("alpha", Some("*.rs"), dir, 10, dir).await?;
    assert_eq!(results.len(), 1);
    assert!(results.iter().all(|path| path.ends_with("match_one.rs")));
    Ok(())
}

#[tokio::test]
async fn run_search_respects_limit() -> anyhow::Result<()> {
    if !rg_available() {
        return Ok(());
    }
    let temp = tempdir().expect("create temp dir");
    let dir = temp.path();
    std::fs::write(dir.join("one.txt"), "alpha one").unwrap();
    std::fs::write(dir.join("two.txt"), "alpha two").unwrap();
    std::fs::write(dir.join("three.txt"), "alpha three").unwrap();

    let results = run_rg_search("alpha", None, dir, 2, dir).await?;
    assert_eq!(results.len(), 2);
    Ok(())
}

#[tokio::test]
async fn run_search_handles_no_matches() -> anyhow::Result<()> {
    if !rg_available() {
        return Ok(());
    }
    let temp = tempdir().expect("create temp dir");
    let dir = temp.path();
    std::fs::write(dir.join("one.txt"), "omega").unwrap();

    let results = run_rg_search("alpha", None, dir, 5, dir).await?;
    assert!(results.is_empty());
    Ok(())
}

#[tokio::test]
async fn grep_content_mode_on_single_file_omits_filename_prefix() -> anyhow::Result<()> {
    if !rg_available() {
        return Ok(());
    }
    let temp = tempdir().expect("create temp dir");
    let dir = temp.path();
    let file = dir.join("gauntlet.txt");
    std::fs::write(&file, "first\nTOKEN_OLD and TOKEN_OLD\nlast\n").unwrap();

    let output = run_rg_command(
        [
            "--color",
            "never",
            "--no-heading",
            "--line-number",
            "TOKEN_OLD",
            file.to_string_lossy().as_ref(),
        ],
        dir,
    )
    .await?;
    let lines = String::from_utf8_lossy(&output.stdout)
        .lines()
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(lines, "2:TOKEN_OLD and TOKEN_OLD");
    Ok(())
}

fn rg_available() -> bool {
    StdCommand::new("rg")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
