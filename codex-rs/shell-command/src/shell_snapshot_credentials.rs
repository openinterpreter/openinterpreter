//! Apply credential and environment policy to captured exports, producing typed render values.
//! The shared literal decoder still validates alias and function bodies without evaluating them.

use super::capture::CapturedSnapshot;
use super::capture::ExportValue;
use super::literals;
use super::render;
use super::render::Export;
use super::render::Value;
use super::render::ValuePart;
use std::collections::HashMap;

/// Environment views used to render protected credential references into a shell snapshot.
pub struct SnapshotCredentialEnvironment<'a> {
    pub original: &'a HashMap<String, String>,
    pub restored: &'a HashMap<String, String>,
    pub configured: &'a HashMap<String, String>,
    pub discovered: &'a HashMap<String, String>,
    pub allowed: &'a HashMap<String, String>,
    /// Applies environment policy to export-only names absent from the captured environment.
    pub is_allowed_unset: &'a dyn Fn(&str) -> bool,
    pub brokered_keys: &'a [String],
    pub brokered_alias_keys: &'a [String],
    pub allowed_brokered_keys: &'a [String],
}

/// Credential-checked replay script and aliases to restore after shell startup.
pub struct PreparedSnapshot {
    pub script: String,
    pub aliases: HashMap<String, String>,
    pub rejected_alias_keys: Vec<String>,
}

/// Prepare a replay script from captured state without changing the captured data.
///
/// Credential policy is applied before rendering. A rejected capture produces no script,
/// including when executable source reconstructs a credential through shell quoting.
pub fn prepare_snapshot_credentials(
    captured: &CapturedSnapshot<'_>,
    environment: SnapshotCredentialEnvironment<'_>,
    mut virtualize_text: impl FnMut(&mut String) -> bool,
) -> Option<PreparedSnapshot> {
    let SnapshotCredentialEnvironment {
        original,
        restored,
        configured,
        discovered,
        allowed,
        is_allowed_unset,
        brokered_keys,
        brokered_alias_keys,
        allowed_brokered_keys,
    } = environment;
    let real_credential_value = |key: &str| restored.get(key).or_else(|| configured.get(key));
    let mut credential_aliases = original
        .iter()
        .filter(|(key, _)| !brokered_keys.contains(key) && !brokered_alias_keys.contains(key))
        .filter_map(|(key, value)| {
            let credential_keys = brokered_keys
                .iter()
                .filter(|credential_key| {
                    real_credential_value(credential_key).is_some_and(|real| {
                        value == real
                            || value.contains(real)
                                && (real.len() >= 16
                                    || discovered.get(*credential_key).is_some_and(|dummy| {
                                        discovered.get(key).is_some_and(|virtualized| {
                                            virtualized != value && virtualized.contains(dummy)
                                        })
                                    }))
                    }) || [
                        original.get(*credential_key),
                        discovered.get(*credential_key),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|dummy| !dummy.is_empty())
                    .any(|dummy| {
                        value == dummy
                            || value.contains(dummy)
                                && (dummy.len() >= 16
                                    || real_credential_value(credential_key).is_some_and(|real| {
                                        // Confirm short spans with the broker's path and pattern rules.
                                        let mut restored = value.replace(dummy, real);
                                        restored != *value
                                            && virtualize_text(&mut restored)
                                            && discovered.get(key) == Some(&restored)
                                    }))
                    })
                })
                .map(String::as_str)
                .collect::<Vec<_>>();
            (!credential_keys.is_empty()).then_some((key.as_str(), credential_keys))
        })
        .filter(|(key, _)| {
            key.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || (index != 0 && byte.is_ascii_digit())
            })
        })
        .collect::<HashMap<_, _>>();
    let credential_alias_assignment = |value: &str, credential_keys: &[&str]| {
        let mut remaining = value;
        let mut assignment = Value::default();
        let mut normalized = String::new();
        while !remaining.is_empty() {
            let next_credential = credential_keys
                .iter()
                .filter_map(|credential_key| {
                    let dummy = discovered.get(*credential_key)?;
                    let real = real_credential_value(credential_key)?;
                    let allowed_key = allowed_brokered_keys
                        .iter()
                        .find(|allowed_key| allowed_key.as_str() == *credential_key)
                        .or_else(|| {
                            allowed_brokered_keys.iter().find(|allowed_key| {
                                real_credential_value(allowed_key) == Some(real)
                            })
                        })?;
                    remaining
                        .find(dummy)
                        .map(|index| (index, allowed_key.as_str(), dummy.as_str()))
                })
                .min_by_key(|(index, _, _)| *index);
            let Some((index, credential_key, dummy)) = next_credential else {
                assignment
                    .parts
                    .push(ValuePart::Literal(remaining.to_string()));
                normalized.push_str(remaining);
                break;
            };
            if index != 0 {
                assignment
                    .parts
                    .push(ValuePart::Literal(remaining[..index].to_string()));
                normalized.push_str(&remaining[..index]);
            }
            assignment.parts.push(ValuePart::Credential {
                key: credential_key.to_string(),
            });
            normalized.push_str(allowed.get(credential_key)?);
            remaining = &remaining[index + dummy.len()..];
        }
        Some((assignment, normalized))
    };
    let credential_alias_is_allowed = |credential_keys: &[&str]| {
        credential_keys.iter().all(|credential_key| {
            real_credential_value(credential_key).is_some_and(|real| {
                allowed_brokered_keys
                    .iter()
                    .any(|allowed_key| real_credential_value(allowed_key) == Some(real))
            })
        })
    };
    let is_disallowed_credential_alias =
        |key: &str, virtualize_text: &mut dyn FnMut(&mut String) -> bool| {
            original.get(key).is_some_and(|value| {
                let mut virtualized = value.clone();
                !virtualize_text(&mut virtualized)
            })
        };
    let mut alias_values = HashMap::new();
    let mut rejected_alias_keys = Vec::new();
    let mut invalid_export = false;
    let exports = captured
        .exports
        .iter()
        .filter_map(|export| {
            let line = export.source.as_ref();
            let key = export.key;

            if (!allowed.contains_key(key)
                && (original.contains_key(key) || !is_allowed_unset(key)))
                || brokered_keys
                    .iter()
                    .any(|credential_key| credential_key == key)
            {
                return None;
            }

            if configured.contains_key(key)
                && let ExportValue::TiedArray(array) = &export.value
            {
                let value = allowed.get(key)?;
                let mut virtualized = value.clone();
                if !virtualize_text(&mut virtualized) || virtualized != *value {
                    invalid_export = true;
                    return Some(Export::Captured(line));
                }
                if credential_aliases.remove(key).is_some() {
                    alias_values.insert(key.to_string(), value.clone());
                }
                // An explicit scalar override replaces captured elements, not the array binding.
                return Some(Export::ArrayBinding {
                    key,
                    declaration: line[..array.span.start].strip_suffix('=')?,
                    suffix: &line[array.span.end..],
                });
            }

            if is_disallowed_credential_alias(key, &mut virtualize_text) {
                return None;
            }

            if brokered_alias_keys.iter().any(|alias_key| alias_key == key) {
                // Source-less aliases keep their live scalar value, including explicit unsets.
                return match &export.value {
                    ExportValue::TiedArray(array) => Some(Export::ArrayBinding {
                        key,
                        declaration: line[..array.span.start].strip_suffix('=')?,
                        suffix: &line[array.span.end..],
                    }),
                    ExportValue::Plain | ExportValue::UnparsedArray => None,
                };
            }

            if let Some(credential_keys) = credential_aliases.remove(key) {
                if !credential_alias_is_allowed(&credential_keys) {
                    rejected_alias_keys.push(key.to_string());
                    return None;
                }
                let declaration = line
                    .split_once('=')
                    .map_or(line.trim_end(), |(prefix, _)| prefix);
                let (assignment, value) =
                    credential_alias_assignment(allowed.get(key)?, &credential_keys)?;
                alias_values.insert(key.to_string(), value);
                if let ExportValue::TiedArray(array) = &export.value {
                    let scalar = array.values.join(array.separator.as_slice());
                    let crosses_element = credential_keys
                        .iter()
                        .flat_map(|key| {
                            [
                                real_credential_value(key),
                                original.get(*key),
                                discovered.get(*key),
                            ]
                        })
                        .flatten()
                        .filter(|credential| !credential.is_empty())
                        .any(|credential| {
                            scalar
                                .windows(credential.len())
                                .enumerate()
                                .any(|(start, bytes)| {
                                    if bytes != credential.as_bytes() {
                                        return false;
                                    }
                                    let mut offset = 0;
                                    !array.values.iter().any(|value| {
                                        let contains = offset <= start
                                            && start + credential.len() <= offset + value.len();
                                        offset += value.len() + array.separator.len();
                                        contains
                                    })
                                })
                        });
                    if crosses_element {
                        invalid_export = true;
                        return Some(Export::Captured(line));
                    }
                    let Some((elements, normalized)) = array
                        .values
                        .iter()
                        .map(|bytes| {
                            let mut text = String::from_utf8(bytes.clone()).ok()?;
                            if !virtualize_text(&mut text) {
                                return None;
                            }
                            credential_alias_assignment(&text, &credential_keys)
                        })
                        .collect::<Option<Vec<_>>>()
                        .map(|values| values.into_iter().unzip::<_, _, Vec<_>, Vec<_>>())
                    else {
                        invalid_export = true;
                        return Some(Export::Captured(line));
                    };
                    // Never replay a known credential that crosses array-element boundaries.
                    if normalized
                        .iter()
                        .map(String::as_bytes)
                        .collect::<Vec<_>>()
                        .join(array.separator.as_slice())
                        != allowed.get(key)?.as_bytes()
                    {
                        invalid_export = true;
                        return Some(Export::Captured(line));
                    }
                    return Some(Export::Array {
                        prefix: &line[..array.span.start],
                        elements,
                        suffix: &line[array.span.end..],
                    });
                }
                if matches!(export.value, ExportValue::UnparsedArray) {
                    invalid_export = true;
                    return Some(Export::Captured(line));
                }
                return Some(Export::Assignment {
                    declaration,
                    value: assignment,
                });
            }

            Some(Export::Captured(line))
        })
        .collect::<Vec<_>>();
    let snapshot = render::render(captured.state, captured.aliases, &exports)?;
    if invalid_export {
        return None;
    }

    for (key, credential_keys) in credential_aliases {
        if !is_disallowed_credential_alias(key, &mut virtualize_text)
            && credential_alias_is_allowed(&credential_keys)
            && let Some((_, value)) = allowed
                .get(key)
                .and_then(|value| credential_alias_assignment(value, &credential_keys))
        {
            // Native capture can deliberately omit readonly exports; retain only their metadata.
            alias_values.insert(key.to_string(), value);
        } else {
            rejected_alias_keys.push(key.to_string());
        }
    }

    let mut virtualized = snapshot.clone();
    if !virtualize_text(&mut virtualized) || virtualized != snapshot {
        return None;
    }

    literals::literal_words(&snapshot, captured.shell_type)?
        .into_iter()
        .all(|word| {
            let mut virtualized = word.clone();
            virtualize_text(&mut virtualized) && virtualized == word
        })
        .then_some(PreparedSnapshot {
            script: snapshot,
            aliases: alias_values,
            rejected_alias_keys,
        })
}
