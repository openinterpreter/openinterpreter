//! Captures bounded, canonical paths for invoked user-owned skills.

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use codex_core::config::Config;
use dirs::home_dir;

const MAX_TRUSTED_SKILLS: usize = 16;
const MAX_TRUSTED_SKILL_PATH_BYTES: usize = 512;
const MAX_TRUSTED_SKILL_PATHS_BYTES: usize = 2_048;
/// Host-owned roots from which invoked skill paths can be verified.
pub(crate) struct TrustedSkillRoots {
    roots: Vec<PathBuf>,
}

impl TrustedSkillRoots {
    pub(crate) fn from_config(config: &Config) -> Self {
        let mut roots = vec![config.codex_home.join("skills").to_path_buf()];
        if let Some(user_home) = home_dir() {
            roots.push(user_home.join(".agents").join("skills"));
        }
        Self { roots }
    }

    pub(crate) fn trusted_skill_path(&self, skill_resource: &str) -> Option<String> {
        let skill_path = Path::new(skill_resource).canonicalize().ok()?;
        if !self.roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|trusted_root| skill_path.starts_with(trusted_root))
        }) {
            return None;
        }

        let path = skill_path.to_str()?.to_owned();
        if path.len() > MAX_TRUSTED_SKILL_PATH_BYTES || !skill_path.is_file() {
            return None;
        }

        Some(path)
    }
}

/// Bounded, deduplicated user-owned skill paths observed in one turn.
#[derive(Default)]
pub(crate) struct TrustedSkillInvocations(BTreeSet<String>);

impl TrustedSkillInvocations {
    pub(crate) fn record(&mut self, path: String) {
        let skills = &mut self.0;
        if skills.contains(&path)
            || skills.len() >= MAX_TRUSTED_SKILLS
            || skills
                .iter()
                .map(String::len)
                .sum::<usize>()
                .saturating_add(path.len())
                > MAX_TRUSTED_SKILL_PATHS_BYTES
        {
            return;
        }
        skills.insert(path);
    }

    pub(crate) fn into_paths(self) -> Vec<String> {
        self.0.into_iter().collect()
    }
}

#[cfg(test)]
#[path = "trusted_skills_tests.rs"]
mod tests;
