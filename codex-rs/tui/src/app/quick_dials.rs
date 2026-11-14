//! Persist numbered prompts and slash commands in the active user configuration.
//! Replace slots owned by the active user config. Clearing must not reveal a
//! shadowed slot from another configuration layer.

use super::App;
use crate::chatwidget::quick_dials::QuickDialEditorTarget;
use crate::config_update::format_config_error;
use crate::legacy_core::config::edit::ConfigEdit;
use crate::legacy_core::config::edit::ConfigEditsBuilder;
use codex_config::ConfigLayerSource;
use codex_config::types::QuickDial;

impl App {
    pub(super) async fn save_quick_dial(&mut self, slot: u8, dial: Option<QuickDial>) {
        let key = slot.to_string();
        let read_only = {
            let mut slot_sources = self
                .config
                .config_layer_stack
                .layers_high_to_low()
                .filter(|layer| {
                    layer
                        .config
                        .get("tui")
                        .and_then(|tui| tui.get("presets"))
                        .and_then(|dials| dials.get(&key))
                        .is_some()
                })
                .map(|layer| &layer.name);
            let outside_user_config = |source: &ConfigLayerSource| {
                !matches!(source, ConfigLayerSource::User { file, .. }
                    if file == &self.local_settings.user_config_path)
            };
            if dial.is_some() {
                slot_sources.next().is_some_and(outside_user_config)
            } else {
                slot_sources.any(outside_user_config)
            }
        };
        if read_only {
            self.chat_widget.add_error_message(format!(
                "Preset {slot} is defined outside your user config. Edit its source configuration instead."
            ));
            self.restore_quick_dial_editor(slot, &dial);
            return;
        }
        let segments = vec!["tui".into(), "presets".into(), key.clone()];
        let edit = match &dial {
            Some(candidate) => {
                if let Err(reason) = candidate.validate() {
                    self.chat_widget.add_error_message(reason.into());
                    self.restore_quick_dial_editor(slot, &dial);
                    return;
                }
                let mut table = toml_edit::Table::new();
                table["input"] = toml_edit::value(&candidate.input);
                // An explicit empty label masks labels inherited from lower layers.
                table["label"] = toml_edit::value(candidate.label.as_deref().unwrap_or_default());
                ConfigEdit::SetPath {
                    segments,
                    value: toml_edit::Item::Table(table),
                }
            }
            None => ConfigEdit::ClearPath { segments },
        };
        if let Err(error) =
            ConfigEditsBuilder::for_config_path(self.local_settings.user_config_path.as_path())
                .with_edits([edit])
                .apply()
                .await
        {
            self.chat_widget.add_error_message(format!(
                "Failed to update preset {slot}: {}",
                format_config_error(&error)
            ));
            self.restore_quick_dial_editor(slot, &dial);
            return;
        }
        let message = match dial {
            Some(mut dial) => {
                dial.label.get_or_insert_default();
                self.local_settings.tui.quick_dials.insert(key, dial);
                format!("Saved preset {slot}.")
            }
            None => {
                self.local_settings.tui.quick_dials.remove(&key);
                format!("Deleted preset {slot}.")
            }
        };
        self.chat_widget.local_settings.tui.quick_dials =
            self.local_settings.tui.quick_dials.clone();
        self.chat_widget.add_info_message(message, /*hint*/ None);
    }

    fn restore_quick_dial_editor(&mut self, slot: u8, dial: &Option<QuickDial>) {
        if let Some(dial) = dial {
            self.chat_widget
                .open_quick_dial_editor(QuickDialEditorTarget::Slot(slot), Some(dial.clone()));
        }
    }
}
