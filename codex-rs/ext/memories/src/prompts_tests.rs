use super::*;
use codex_utils_absolute_path::AbsolutePathBuf;
use pretty_assertions::assert_eq;
use tempfile::tempdir;
use tokio::fs as tokio_fs;

#[tokio::test]
async fn build_memory_tool_developer_instructions_renders_embedded_template() {
    let temp = tempdir().unwrap();
    let codex_home = AbsolutePathBuf::from_absolute_path(temp.path()).unwrap();
    let memories_dir = codex_home.join("memories");
    tokio_fs::create_dir_all(&memories_dir).await.unwrap();
    tokio_fs::write(
        memories_dir.join("memory_summary.md"),
        "Short memory summary for tests.",
    )
    .await
    .unwrap();

    let instructions = build_memory_tool_developer_instructions(&codex_home, MemoryVersion::V1)
        .await
        .unwrap();

    assert!(instructions.contains(&format!(
        "- {}/memory_summary.md (already provided below; do NOT open again)",
        memories_dir.display()
    )));
    assert!(instructions.contains("Short memory summary for tests."));
    assert_eq!(
        instructions
            .matches("========= MEMORY_SUMMARY BEGINS =========")
            .count(),
        1
    );
}

#[tokio::test]
async fn v2_reads_only_its_own_summary_without_falling_back_to_v1()
-> Result<(), Box<dyn std::error::Error>> {
    let home = tempdir()?;
    let codex_home = AbsolutePathBuf::from_absolute_path(home.path())?;
    let v1 = codex_home.join("memories");
    let v2 = codex_home.join("memories_v2");
    tokio_fs::create_dir_all(&v1).await?;
    tokio_fs::write(v1.join("memory_summary.md"), "v1\nlegacy content").await?;
    assert_eq!(
        build_memory_tool_developer_instructions(&codex_home, MemoryVersion::V2).await,
        None
    );
    tokio_fs::create_dir_all(&v2).await?;
    tokio_fs::write(v2.join("memory_summary.md"), "v1\nnew pipeline content").await?;
    let instructions = build_memory_tool_developer_instructions(&codex_home, MemoryVersion::V2)
        .await
        .expect("v2 instructions");
    assert!(instructions.contains("new pipeline content"));
    assert!(!instructions.contains("legacy content"));
    assert!(instructions.contains("do not retrieve history speculatively"));
    assert!(instructions.contains(&format!("{}/rollout_summaries/", v2.display())));
    assert!(
        build_memory_tool_developer_instructions(&codex_home, MemoryVersion::V1)
            .await
            .expect("v1 instructions")
            .contains("legacy content")
    );
    Ok(())
}
