use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::ExecutableIdentity;
use super::executable_identity;
use super::managed_codex_bin;
use super::managed_codex_bin_for_package_dir_and_current_exe;
use super::parse_codex_version;

#[test]
fn parses_codex_cli_version_output() {
    assert_eq!(
        parse_codex_version("codex 1.2.3\n").expect("version"),
        "1.2.3"
    );
}

#[test]
fn rejects_malformed_codex_cli_version_output() {
    assert!(parse_codex_version("codex\n").is_err());
}

#[tokio::test]
async fn executable_identity_uses_path_and_binary_contents() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let executable = directory.path().join("codex");
    // Span multiple reads, including a partial final buffer, and preserve the
    // digest stored by older clients that hashed the complete file in memory.
    let mut bytes: Vec<u8> = (0..200_003).map(|index| (index % 251) as u8).collect();
    for contents in [&bytes[..], &[][..]] {
        std::fs::write(&executable, contents).expect("write executable");
        assert_eq!(
            executable_identity(&executable).await.expect("identity"),
            ExecutableIdentity {
                digest: *blake3::hash(contents).as_bytes(),
                path_digest: Some(super::path_digest(
                    &std::fs::canonicalize(&executable).expect("canonical executable"),
                )),
            }
        );
    }
    let copy = directory.path().join("codex-copy");
    std::fs::copy(&executable, &copy).expect("copy executable");
    let identity = executable_identity(&executable).await.expect("identity");
    let copy_identity = executable_identity(&copy).await.expect("copy identity");
    assert_ne!(identity, copy_identity);
    assert!(identity.same_contents(&copy_identity));
    std::fs::write(&executable, &bytes).expect("write executable");
    let old = executable_identity(&executable).await.expect("identity");
    bytes[100_000] ^= 1;
    std::fs::write(&executable, bytes).expect("replace executable");
    assert_ne!(
        executable_identity(&executable)
            .await
            .expect("new identity"),
        old
    );
}

#[test]
fn managed_codex_bin_prefers_package_metadata() {
    let codex_home = TempDir::new().expect("codex home");
    let current_dir = codex_home.path().join("packages/standalone/current");
    let bin_dir = current_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create package bin");
    let managed_codex = bin_dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&managed_codex, "").expect("write managed codex");
    std::fs::write(
        current_dir.join("codex-package.json"),
        format!(
            "{{\"managedCodex\":\"bin/{}\"}}",
            if cfg!(windows) { "codex.exe" } else { "codex" }
        ),
    )
    .expect("write package metadata");

    assert_eq!(
        managed_codex_bin(codex_home.path()),
        managed_codex
            .canonicalize()
            .expect("canonical managed codex")
    );
}

#[test]
fn managed_codex_bin_uses_package_entrypoint_when_managed_codex_is_missing() {
    let codex_home = TempDir::new().expect("codex home");
    let current_dir = codex_home.path().join("packages/standalone/current");
    let bin_dir = current_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create package bin");
    let entrypoint = bin_dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&entrypoint, "").expect("write package entrypoint");
    std::fs::write(
        current_dir.join("codex-package.json"),
        format!(
            "{{\"entrypoint\":\"bin/{}\"}}",
            if cfg!(windows) { "codex.exe" } else { "codex" }
        ),
    )
    .expect("write package metadata");

    assert_eq!(
        managed_codex_bin(codex_home.path()),
        entrypoint.canonicalize().expect("canonical entrypoint")
    );
}

#[test]
fn managed_codex_bin_falls_back_to_legacy_current_path() {
    let codex_home = TempDir::new().expect("codex home");

    assert_eq!(
        managed_codex_bin(codex_home.path()),
        codex_home
            .path()
            .join("packages/standalone/current")
            .join(if cfg!(windows) { "codex.exe" } else { "codex" })
    );
}

#[test]
fn managed_codex_bin_falls_back_to_sibling_binary_for_source_tree_debug() {
    let codex_home = TempDir::new().expect("codex home");
    let debug_dir = TempDir::new().expect("debug dir");
    let current_exe = debug_dir.path().join(if cfg!(windows) {
        "interpreter.exe"
    } else {
        "interpreter"
    });
    let sibling_codex = debug_dir
        .path()
        .join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&current_exe, "").expect("write current exe");
    std::fs::write(&sibling_codex, "").expect("write sibling codex");

    assert_eq!(
        managed_codex_bin_for_package_dir_and_current_exe(
            codex_home.path(),
            /*package_dir*/ None,
            Some(&current_exe),
        ),
        sibling_codex
    );
}

#[test]
fn managed_codex_bin_prefers_current_package_layout_over_state_home() {
    let codex_home = TempDir::new().expect("codex home");
    let package_dir = TempDir::new().expect("package dir");
    let bin_dir = package_dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).expect("create package bin");
    let managed_codex = bin_dir.join(if cfg!(windows) { "codex.exe" } else { "codex" });
    std::fs::write(&managed_codex, "").expect("write managed codex");
    std::fs::write(
        package_dir.path().join("codex-package.json"),
        format!(
            "{{\"managedCodex\":\"bin/{}\"}}",
            if cfg!(windows) { "codex.exe" } else { "codex" }
        ),
    )
    .expect("write package metadata");
    assert_eq!(
        managed_codex_bin_for_package_dir_and_current_exe(
            codex_home.path(),
            Some(package_dir.path()),
            /*current_exe*/ None,
        ),
        managed_codex
            .canonicalize()
            .expect("canonical managed codex")
    );
}
