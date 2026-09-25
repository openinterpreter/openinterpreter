//! Core's mandatory-denial validation, shared without resolving paths on the controller.
//! Checks raw concrete grants; policy application and sandbox enforcement remain caller-owned.

use super::FileSystemAccessMode;
use super::FileSystemPath;
use super::FileSystemSandboxEntry;
use super::FileSystemSandboxKind;
use super::FileSystemSandboxPolicy;
use super::FileSystemSandboxPolicyContext;
use super::FileSystemSpecialPath;
use super::InvalidDenyReadGlobBehavior;
use super::PreparedReadDenyMatcher;
use codex_utils_path_uri::PathConvention;
use codex_utils_path_uri::PathUri;

/// Prepared mandatory rules reused when a caller replaces its selected profile.
pub struct DenyReadValidator {
    required_entries: Vec<FileSystemSandboxEntry>,
    matcher: Option<PreparedReadDenyMatcher>,
}

/// Details needed to retain the caller's existing sourced constraint diagnostics.
#[derive(Debug, PartialEq, Eq)]
pub enum DenyReadViolation {
    MissingRequiredDeny,
    ReadablePath(PathUri),
}

impl DenyReadValidator {
    /// Prepares the strict matcher once, using the supplied executor facts.
    pub fn new(
        required: &FileSystemSandboxPolicy,
        context: &FileSystemSandboxPolicyContext<'_>,
    ) -> Result<Self, String> {
        let matcher = has_context_read_denials(required, context)
            .then(|| {
                required
                    .prepare_deny_read_matcher(context, InvalidDenyReadGlobBehavior::ReturnError)
            })
            .transpose()?;
        Ok(Self {
            required_entries: required.entries.clone(),
            matcher,
        })
    }

    /// Preserves Core's entry-presence and raw readable-path checks.
    ///
    /// Symbolic entries are not expanded. An equally specific deny does not
    /// exempt a concrete readable grant from this validation. Native callers
    /// retain their native-path conversion boundary before passing the policy.
    pub fn validate(
        &self,
        selected: &FileSystemSandboxPolicy,
        context: &FileSystemSandboxPolicyContext<'_>,
    ) -> Result<(), DenyReadViolation> {
        let missing_required_deny = self
            .required_entries
            .iter()
            .any(|entry| !selected.entries.contains(entry));
        let violating_root = selected
            .entries
            .iter()
            .filter(|entry| entry.access.can_read())
            .find_map(|entry| {
                let FileSystemPath::Path { path } = &entry.path else {
                    return None;
                };
                if path.infer_path_convention() != context.cwd.infer_path_convention() {
                    return None;
                }
                self.matcher
                    .as_ref()
                    .is_some_and(|matcher| {
                        FileSystemSandboxPolicy::matches_prepared_read_deny(path, context, matcher)
                    })
                    .then_some(path)
            });
        // Core reports the first conflicting path even when a required entry is missing too.
        if let Some(path) = violating_root {
            return Err(DenyReadViolation::ReadablePath(path.clone()));
        }
        if missing_required_deny {
            return Err(DenyReadViolation::MissingRequiredDeny);
        }
        Ok(())
    }
}

pub(super) fn has_context_read_denials(
    policy: &FileSystemSandboxPolicy,
    context: &FileSystemSandboxPolicyContext<'_>,
) -> bool {
    policy.kind == FileSystemSandboxKind::Restricted
        && policy.entries.iter().any(|entry| {
            entry.access == FileSystemAccessMode::Deny
                && !matches!(&entry.path,
                    FileSystemPath::Special { value: FileSystemSpecialPath::SlashTmp }
                        if context.cwd.infer_path_convention() != Some(PathConvention::Posix)
                )
        })
}
