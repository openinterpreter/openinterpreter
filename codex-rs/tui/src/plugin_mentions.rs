#[cfg(feature = "extended-mentions")]
pub(crate) type PluginMentionSummary = codex_plugin::PluginCapabilitySummary;

#[cfg(not(feature = "extended-mentions"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PluginMentionSummary;
