use super::*;
use crate::permissions::FileSystemAccessMode;
use pretty_assertions::assert_eq;

#[test]
fn local_aliases_keep_read_only_and_deny_overrides() -> anyhow::Result<()> {
    let directory = tempfile::tempdir_in("/tmp")?;
    let logical = PathUri::from_host_native_path(directory.path())?;
    let physical = PathUri::from_host_native_path(directory.path().canonicalize()?)?;
    let context = FileSystemSandboxPolicyContext {
        cwd: &physical,
        workspace_roots: std::slice::from_ref(&physical),
        user_home_dir: None,
        temporary_directories: None,
    };
    for writable in [&logical, &physical] {
        for restricted in [&logical, &physical] {
            for access in [FileSystemAccessMode::Read, FileSystemAccessMode::Deny] {
                let policy = FileSystemSandboxPolicy::restricted(vec![
                    FileSystemSandboxEntry::new(
                        writable.clone().into(),
                        FileSystemAccessMode::Write,
                    ),
                    FileSystemSandboxEntry::new(restricted.join("protected")?.into(), access),
                ]);
                let matching = policy.prepare_local_matching(&context)?;
                for spelling in [&logical, &physical] {
                    let protected = spelling.join("protected/missing/file")?;
                    assert_eq!(
                        (
                            matching.can_write_path(&spelling.join("missing/file")?)?,
                            matching.can_write_path(&protected)?,
                            policy.resolve_access_for_local_path_with_cwd(
                                protected.to_abs_path()?.as_path(),
                                directory.path(),
                            ),
                            matching.can_write_path(&spelling.join(".codex/config.toml")?)?,
                        ),
                        (true, false, access, false),
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
fn equivalent_alias_entries_keep_restrictive_precedence() -> anyhow::Result<()> {
    let directory = tempfile::tempdir_in("/tmp")?;
    let logical = PathUri::from_host_native_path(directory.path())?;
    let physical = PathUri::from_host_native_path(directory.path().canonicalize()?)?;
    let policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry::new(physical.clone().into(), FileSystemAccessMode::Write),
        FileSystemSandboxEntry::new(logical.into(), FileSystemAccessMode::Deny),
    ]);
    assert_eq!(
        policy.resolve_access_for_local_path_with_cwd(
            physical.join("missing")?.to_abs_path()?.as_path(),
            directory.path(),
        ),
        FileSystemAccessMode::Deny,
    );
    Ok(())
}

#[test]
fn local_metadata_case_aliases_respect_filesystem_and_explicit_grants() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let parent = PathUri::from_host_native_path(directory.path().canonicalize()?)?;
    let workspace = parent.join("workspace")?.to_abs_path()?;
    std::fs::create_dir(&workspace)?;
    std::fs::create_dir(workspace.join(".git"))?;
    let case_insensitive = workspace.join(".GIT").exists();
    let root = PathUri::from_host_native_path(&workspace)?;
    let context = FileSystemSandboxPolicyContext {
        cwd: &root,
        workspace_roots: std::slice::from_ref(&root),
        user_home_dir: None,
        temporary_directories: None,
    };
    let policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry::new(parent.clone().into(), FileSystemAccessMode::Write),
        FileSystemSandboxEntry::new(root.clone().into(), FileSystemAccessMode::Write),
    ]);
    let matching = policy.prepare_local_matching(&context)?;
    for relative in [
        ".GIT/config",
        ".CoDeX/config.toml",
        ".AGENTS/new",
        ".AWS/config",
    ] {
        let path = root.join(relative)?;
        assert_eq!(
            (
                matching.can_write_path(&path)?,
                policy.can_write_path(&path, &context),
            ),
            (!case_insensitive, true),
            "local aliases must not change executor-independent matching: {relative}",
        );
    }
    let root_alias = parent.join("WORKSPACE")?;
    for relative in [".GIT/config", ".git/config", ".CoDeX/config.toml"] {
        let path = root_alias.join(relative)?;
        assert_eq!(
            (
                matching.can_write_path(&path)?,
                policy.can_write_path(&path, &context),
            ),
            (!case_insensitive, true),
            "a broader grant must not bypass metadata protection via a root alias: {relative}",
        );
    }
    for name in [".git", ".GIT"] {
        let mut explicitly_writable = policy.clone();
        explicitly_writable
            .entries
            .push(FileSystemSandboxEntry::new(
                root.join(name)?.into(),
                FileSystemAccessMode::Write,
            ));
        let matching = explicitly_writable.prepare_local_matching(&context)?;
        for spelling in [&root, &root_alias] {
            assert!(
                matching.can_write_path(&spelling.join(".GIT/config")?)?,
                "explicit metadata grant must remain effective: {name}",
            );
        }
    }
    for (grant, restriction, candidate) in [
        (".git", ".GIT/private", ".git/private"),
        (".GIT", ".git/private", ".GIT/private"),
    ] {
        for access in [FileSystemAccessMode::Read, FileSystemAccessMode::Deny] {
            let mut restricted = policy.clone();
            restricted.entries.extend([
                FileSystemSandboxEntry::new(root.join(grant)?.into(), FileSystemAccessMode::Write),
                FileSystemSandboxEntry::new(root.join(restriction)?.into(), access),
            ]);
            assert_eq!(
                restricted
                    .prepare_local_matching(&context)?
                    .can_write_path(&root.join(candidate)?)?,
                !case_insensitive,
                "narrower metadata restrictions must retain precedence: {restriction}",
            );
        }
    }
    let missing_root = root.join("missing")?;
    let missing_policy = FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry::new(
        missing_root.clone().into(),
        FileSystemAccessMode::Write,
    )]);
    assert_eq!(
        missing_policy
            .prepare_local_matching(&context)?
            .can_write_path(&missing_root.join(".GIT/config")?)?,
        !case_insensitive,
    );
    assert!(matching.can_write_path(&root.join("nested/.GIT/config")?)?);
    Ok(())
}
