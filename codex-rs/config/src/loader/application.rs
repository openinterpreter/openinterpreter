//! Reads application requirements independently of user, project, and cloud configuration.
//! Captured local layers retain their positions around cloud layers during composition.

use crate::ApplicationRequirementsToml;
use crate::CloudRequirementsTomlBundle;
use crate::LoaderOverrides;
use crate::RequirementsLayerEntry;
use crate::compose_requirements;
use codex_file_system::ExecutorFileSystem;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::io;
use std::path::Path;
use toml::Value;

/// Local application sources captured without waiting for a cloud request.
#[derive(Clone, Debug, Default)]
pub struct LocalApplicationRequirements {
    system: Option<RequirementsLayerEntry>,
    mdm: Option<RequirementsLayerEntry>,
    ignore_managed: bool,
}

impl LocalApplicationRequirements {
    /// Composes system, cloud, and MDM application policy in the normal requirements order.
    pub fn compose(
        &self,
        cloud: CloudRequirementsTomlBundle,
    ) -> io::Result<Option<ApplicationRequirementsToml>> {
        if self.ignore_managed {
            return Ok(None);
        }
        if !cloud.enterprise_managed.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cloud application requirements require an explicit Codex home base directory",
            ));
        }
        self.compose_layers(Vec::new())
    }

    /// Composes cloud requirements against the same Codex home as normal config loading.
    pub fn compose_with_cloud_base_dir(
        &self,
        cloud: CloudRequirementsTomlBundle,
        codex_home: &Path,
    ) -> io::Result<Option<ApplicationRequirementsToml>> {
        if self.ignore_managed {
            return Ok(None);
        }
        let base_dir = AbsolutePathBuf::from_absolute_path(codex_home)?;
        self.compose_layers(cloud.into_layers(&base_dir))
    }

    fn compose_layers(
        &self,
        cloud_layers: Vec<RequirementsLayerEntry>,
    ) -> io::Result<Option<ApplicationRequirementsToml>> {
        if self.ignore_managed {
            return Ok(None);
        }
        let mut layers = self.system.clone().into_iter().collect::<Vec<_>>();
        for layer in cloud_layers {
            layers.extend(application_layer(layer)?);
        }
        layers.extend(self.mdm.clone());
        Ok(compose_requirements(layers)?
            .and_then(|requirements| requirements.into_toml().application))
    }
}

/// Reads only the local sources that can set application requirements.
/// macOS policy reads require a successful preference refresh before using cached values.
pub async fn load_local_application_requirements(
    fs: &dyn ExecutorFileSystem,
    overrides: &LoaderOverrides,
) -> io::Result<LocalApplicationRequirements> {
    if overrides.ignore_managed_requirements {
        return Ok(LocalApplicationRequirements {
            ignore_managed: true,
            ..Default::default()
        });
    }
    let system_path = super::system_requirements_toml_file_with_overrides(overrides)?;
    let system = super::load_requirements_toml(fs, &system_path)
        .await?
        .map(application_layer)
        .transpose()?
        .flatten();
    #[cfg(target_os = "macos")]
    let mdm = {
        if overrides.macos_managed_config_requirements_base64.is_none() {
            tokio::task::spawn_blocking(super::macos::synchronize_managed_preferences)
                .await
                .map_err(io::Error::other)??;
        }
        super::macos::load_managed_admin_requirements_layer(
            overrides
                .macos_managed_config_requirements_base64
                .as_deref(),
        )
        .await?
        .map(application_layer)
        .transpose()?
        .flatten()
    };
    #[cfg(not(target_os = "macos"))]
    let mdm = None;
    let local = LocalApplicationRequirements {
        system,
        mdm,
        ignore_managed: false,
    };
    // Validate local requirements before cloud loading.
    local.compose(CloudRequirementsTomlBundle::default())?;
    Ok(local)
}

fn application_layer(layer: RequirementsLayerEntry) -> io::Result<Option<RequirementsLayerEntry>> {
    let (source, mut value, base_dir) = layer.into_raw_parts()?;
    let Some(application) = value
        .as_table_mut()
        .and_then(|table| table.remove("application"))
    else {
        return Ok(None);
    };
    let entry = RequirementsLayerEntry::from_toml_value(
        source,
        Value::Table(toml::map::Map::from_iter([(
            "application".into(),
            application,
        )])),
    );
    Ok(Some(match base_dir {
        Some(base_dir) => entry.with_base_dir(base_dir),
        None => entry,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CloudRequirementsFragment;

    #[test]
    fn cloud_application_policy_requires_explicit_home_and_retains_layer_base() {
        let codex_home = tempfile::tempdir().expect("Codex home");
        let cloud = CloudRequirementsTomlBundle {
            enterprise_managed: vec![CloudRequirementsFragment {
                id: "policy".to_string(),
                name: "Cloud policy".to_string(),
                contents: "[application.network.domains]\n\"allowed.example\" = \"allow\""
                    .to_string(),
            }],
        };
        let local = LocalApplicationRequirements::default();
        assert_eq!(
            local.compose(cloud.clone()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let application = local
            .compose_with_cloud_base_dir(cloud, codex_home.path())
            .expect("valid cloud policy")
            .expect("network policy");
        assert_eq!(application.network.expect("network").domains.len(), 1);

        let base_dir = AbsolutePathBuf::from_absolute_path(codex_home.path()).expect("home");
        let layer = RequirementsLayerEntry::from_toml(
            crate::RequirementSource::EnterpriseManaged {
                id: "policy".into(),
                name: "Cloud policy".into(),
            },
            "[application.network]",
        )
        .with_base_dir(base_dir.clone());
        let retained = application_layer(layer)
            .expect("parse application policy")
            .expect("application layer");
        assert_eq!(
            retained.into_raw_parts().expect("raw parts").2,
            Some(base_dir)
        );
    }
}
