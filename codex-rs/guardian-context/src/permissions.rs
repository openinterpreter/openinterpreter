//! Sync permission evidence formatting. The host resolves the reviewed environment
//! and its filesystem policy; this section neither resolves nor relaxes restrictions.

use crate::ContextSection;
use crate::SectionContributor;
use crate::SectionError;
use crate::SectionInput;
use crate::SectionScope;

/// Display paths and globs denied by the reviewed environment's active policy.
/// These are evidence strings, not filesystem paths to resolve in the reviewer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PermissionContext {
    pub denied_paths: Vec<String>,
    pub denied_globs: Vec<String>,
}

pub(crate) struct PermissionContextSection;

impl SectionContributor for PermissionContextSection {
    fn scope(&self) -> SectionScope {
        SectionScope::SyncOnly
    }

    fn contribute(&self, input: &SectionInput<'_>) -> Result<Option<ContextSection>, SectionError> {
        let Some(permissions) = input.permissions else {
            return Ok(None);
        };
        let entries = permissions
            .denied_paths
            .iter()
            .map(|path| format!("- path `{path}`"))
            .chain(
                permissions
                    .denied_globs
                    .iter()
                    .map(|glob| format!("- glob `{glob}`")),
            )
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return Ok(None);
        }
        Ok(Some(ContextSection::PermissionContext {
            items: vec![
                "\n>>> PARENT TURN PERMISSION CONTEXT START\n".into(),
                format!(
                    "The parent turn's active permission profile denies reading these paths/globs. These are policy restrictions; do not approve escalation whose purpose is to read them.\n{}\n",
                    entries.join("\n")
                ),
                ">>> PARENT TURN PERMISSION CONTEXT END\n".into(),
            ],
        }))
    }
}
