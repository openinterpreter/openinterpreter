use super::CredentialBrokerState;
use super::MIN_EMBEDDED_CREDENTIAL_LENGTH;
use super::env_key_matches;
use super::env_value;
use super::prioritized_credentials;
use super::providers;
use super::registry::BrokeredCredentialProvider;
use super::registry::is_builtin_shaped_credential;
use super::replacement::Replacements;
use std::collections::HashMap;
use std::path::Path;
use url::Position;
use url::Url;

pub(super) struct KnownCredentialMatch<'a> {
    pub(super) range: std::ops::Range<usize>,
    pub(super) env_var: &'a str,
    pub(super) provider: BrokeredCredentialProvider,
    pub(super) real_value: &'a str,
    pub(super) value: &'a str,
}

pub(super) fn known_credential_matches<'a>(
    state: &'a CredentialBrokerState,
    text: &str,
    source_env: &'a HashMap<String, String>,
) -> Vec<KnownCredentialMatch<'a>> {
    let mut matches = Vec::new();
    let mut add = |env_var: &'a str,
                   provider: &BrokeredCredentialProvider,
                   real_value: &'a str,
                   value: &'a str| {
        if value.is_empty() {
            return;
        }
        let ranges = if text == value {
            std::iter::once(0..value.len()).collect()
        } else if value.len() >= MIN_EMBEDDED_CREDENTIAL_LENGTH
            && (is_builtin_shaped_credential(real_value)
                || matches!(provider, BrokeredCredentialProvider::Builtin(_)))
        {
            text.match_indices(value)
                .map(|(start, _)| start..start + value.len())
                .collect()
        } else if let BrokeredCredentialProvider::Configured(provider) = provider {
            // Configured short values retain their pattern boundaries and path exclusions.
            provider
                .credential_value_match_ranges(text, value)
                .into_iter()
                .filter(|range| {
                    value.len() >= MIN_EMBEDDED_CREDENTIAL_LENGTH
                        || !is_operational_path_match(text, range.start, range.end)
                        // A complete longer configured token still needs its own source permission.
                        && !provider.find_discoverable_credentials(text).any(|candidate| {
                            candidate.range.start <= range.start && candidate.range.end > range.end
                        })
                })
                .collect()
        } else {
            Vec::new()
        };
        matches.extend(ranges.into_iter().map(|range| KnownCredentialMatch {
            range,
            env_var,
            provider: provider.clone(),
            real_value,
            value,
        }));
    };
    for credential in &state.credentials {
        for value in [&credential.real_value, &credential.dummy_value] {
            add(
                &credential.env_var,
                &credential.provider,
                &credential.real_value,
                value,
            );
        }
    }
    for provider in super::credential_provider_definitions(state) {
        let owns_key = |key: &str| match &provider {
            BrokeredCredentialProvider::Builtin(provider) => {
                provider.sources().iter().any(|source| {
                    source
                        .env_vars
                        .iter()
                        .any(|source| env_key_matches(source, key))
                })
            }
            BrokeredCredentialProvider::Configured(provider) => provider
                .config
                .env
                .iter()
                .any(|source| env_key_matches(source, key)),
        };
        for owner in &state.credential_owners {
            if owns_key(&owner.env_var) {
                add(
                    &owner.env_var,
                    &provider,
                    &owner.real_value,
                    &owner.real_value,
                );
            }
        }
        for key in source_env.keys() {
            if owns_key(key)
                && let Some(real) =
                    super::brokerable_credential_value(source_env, state, key, &provider)
            {
                add(key, &provider, real, real);
            }
        }
    }
    for credential in &state.credentials {
        matches.extend(
            credential
                .generated_dummy_ranges(text)
                .into_iter()
                .map(|range| KnownCredentialMatch {
                    range,
                    env_var: &credential.env_var,
                    provider: credential.provider.clone(),
                    real_value: &credential.real_value,
                    value: &credential.dummy_value,
                }),
        );
    }
    // Longest exact identities win over overlapping shorter identities, as in replacement.
    matches.sort_unstable_by_key(|matched| {
        (std::cmp::Reverse(matched.range.len()), matched.range.start)
    });
    let mut selected = Vec::<KnownCredentialMatch<'_>>::new();
    for matched in matches {
        if selected.iter().any(|known| {
            known.range.start < matched.range.end
                && matched.range.start < known.range.end
                && (known.range != matched.range
                    || known.provider.same_provider(&matched.provider)
                        && env_key_matches(known.env_var, matched.env_var)
                        && known.real_value == matched.real_value)
        }) {
            continue;
        }
        selected.push(matched);
    }
    selected
}

pub(super) fn mask_known_credentials(text: &str, matches: &[KnownCredentialMatch<'_>]) -> String {
    let mut uncovered = text.to_string();
    for matched in matches {
        uncovered.replace_range(matched.range.clone(), &"\0".repeat(matched.range.len()));
    }
    uncovered
}

pub(super) fn virtualize_text(
    state: &CredentialBrokerState,
    output: &mut String,
    env: &HashMap<String, String>,
) -> bool {
    if !state.enabled {
        return true;
    }

    let allowed_keys = state.environment(env).credential_keys;
    let credentials = prioritized_credentials(state, env);
    let text = output.as_str();
    let source_env = HashMap::new();
    let known = known_credential_matches(state, text, &source_env);
    let mut replacements = Replacements::default();
    let binding_env = state.context.with_fallbacks(env);
    let mut allowed = true;
    for credential in &credentials {
        let ranges = known
            .iter()
            .filter(|matched| {
                matched.provider.same_provider(&credential.provider)
                    && env_key_matches(matched.env_var, &credential.env_var)
                    && matched.real_value == credential.real_value
                    && (matched.value == credential.real_value
                        || matched.value == credential.dummy_value)
            })
            .map(|matched| matched.range.clone())
            .collect::<Vec<_>>();
        if ranges.is_empty() {
            continue;
        }

        let replacement = credentials.iter().copied().find(|candidate| {
            candidate.provider.same_provider(&credential.provider)
                && candidate.real_value == credential.real_value
                && (allowed_keys.iter().any(|key| {
                    env_key_matches(key, &candidate.env_var)
                        && env_value(env, key) == Some(candidate.dummy_value.as_str())
                }) || state.credential_aliases.iter().any(|alias| {
                    env_value(env, &alias.env_var) == Some(alias.dummy_value.as_str())
                        && (alias.dummy_value == candidate.dummy_value
                            || candidate.contains_embedded_value(
                                &alias.dummy_value,
                                &candidate.dummy_value,
                            ))
                        && !state.credentials.iter().any(|other| {
                            other.dummy_value != candidate.dummy_value
                                && (alias.dummy_value == other.dummy_value
                                    || other.contains_embedded_value(
                                        &alias.dummy_value,
                                        &other.dummy_value,
                                    ))
                        })
                }))
        });
        if replacement.is_none() {
            allowed = false;
        }
        replacements.add(
            ranges,
            replacement.map_or("", |candidate| candidate.dummy_value.as_str()),
            replacement,
        );
    }

    let original = text;
    let mut uncovered = replacements.masked(original);
    let text = &mut uncovered;
    // Startup can copy a supported credential before unsetting its source variable.
    for provider in providers::credential_providers() {
        for prefix in provider.credential_prefixes {
            let mut offset = 0;
            while let Some(position) = text[offset..].find(prefix) {
                let start = offset + position;
                if let Some(length) = state
                    .credentials
                    .iter()
                    .flat_map(|credential| {
                        [&credential.real_value, &credential.dummy_value]
                            .map(|value| (credential, value))
                    })
                    .filter(|(_, value)| text[start..].starts_with(value.as_str()))
                    .filter(|(credential, value)| {
                        value.len() >= MIN_EMBEDDED_CREDENTIAL_LENGTH
                            || matches!(&credential.provider, BrokeredCredentialProvider::Configured(provider)
                                if provider.credential_value_match_ranges(text, value)
                                    .iter().any(|range| range.start == start
                                        && !is_operational_path_match(text, range.start, range.end)))
                    })
                    .map(|(_, value)| value.len())
                    .max()
                {
                    offset = start + length;
                    continue;
                }
                let candidate = builtin_credential_candidate(provider, text, start);
                let length = candidate.len();
                if provider
                    .ignored_credential_prefixes
                    .iter()
                    .any(|prefix| candidate.starts_with(prefix))
                    && provider
                        .credential_watermark
                        .is_none_or(|watermark| !candidate.contains(watermark))
                {
                    let known_length = env
                        .values()
                        .filter(|known| {
                            known.len() >= provider.minimum_credential_len
                                && candidate
                                    .strip_prefix(known.as_str())
                                    .is_some_and(|suffix| {
                                        provider.credential_prefixes.iter().any(|prefix| {
                                            suffix.match_indices(prefix).any(|(offset, _)| {
                                                suffix.len() - offset
                                                    >= provider.minimum_credential_len
                                            })
                                        })
                                    })
                        })
                        .map(String::len)
                        .min();
                    let embedded_supported = provider
                        .credential_prefixes
                        .iter()
                        .flat_map(|prefix| candidate.match_indices(prefix))
                        .filter_map(|(offset, _)| {
                            (offset > 0
                                && candidate.len() - offset >= provider.minimum_credential_len
                                && !provider
                                    .ignored_credential_prefixes
                                    .iter()
                                    .any(|ignored| candidate[offset..].starts_with(ignored)))
                            .then_some(offset)
                        })
                        .min();
                    offset = start
                        + known_length
                            .into_iter()
                            .chain(embedded_supported)
                            .min()
                            .unwrap_or(length);
                    continue;
                }
                let end = start + length;
                let credential = &text[start..end];
                let ignored_credential_match =
                    ignored_credential_match(provider, text, start, credential);
                if length >= provider.minimum_credential_len && !ignored_credential_match {
                    replacements.add(std::iter::once(start..end), "", /*dummy*/ None);
                    text.replace_range(start..end, &"\0".repeat(length));
                    offset = end;
                    allowed = false;
                } else {
                    offset = start
                        + if ignored_credential_match {
                            prefix.len()
                        } else {
                            length
                        };
                }
            }
        }
    }

    for provider in &state.configured_providers {
        if provider.host_binding(&binding_env).is_none() {
            continue;
        }
        let mut matches = provider
            .find_credential_matches(text)
            .map(|matched| (matched.start, matched.end))
            .collect::<Vec<_>>();
        matches.sort_unstable_by_key(|(start, end)| (std::cmp::Reverse(end - start), *start));
        matches.dedup();
        let mut removed_ranges = Vec::<(usize, usize)>::new();
        for (start, end) in matches {
            let credential = &text[start..end];
            if credential.contains('\0')
                || !provider
                    .credential_value_match_ranges(original, credential)
                    .contains(&(start..end))
                || is_operational_path_match(original, start, end)
                || state
                    .credentials
                    .iter()
                    .flat_map(|known| [&known.real_value, &known.dummy_value])
                    .flat_map(|known| {
                        text.match_indices(known)
                            .map(move |(index, _)| (index, known))
                    })
                    .any(|(index, known)| index <= start && end <= index + known.len())
                || provider
                    .config
                    .env
                    .iter()
                    .any(|key| env_value(env, key).is_some_and(|value| value == credential))
                || removed_ranges.iter().any(|(removed_start, removed_end)| {
                    start < *removed_end && *removed_start < end
                })
            {
                continue;
            }
            removed_ranges.push((start, end));
            allowed = false;
        }
        removed_ranges.sort_unstable_by_key(|(start, _)| std::cmp::Reverse(*start));
        for (start, end) in removed_ranges {
            replacements.add(std::iter::once(start..end), "", /*dummy*/ None);
            text.replace_range(start..end, &"\0".repeat(end - start));
        }
    }

    replacements.render(output);
    allowed
}

pub(super) fn is_operational_path_match(text: &str, start: usize, end: usize) -> bool {
    let is_value_boundary = |character: char| {
        character.is_ascii_whitespace() || matches!(character, '"' | '\'' | '=' | '`')
    };
    let value_start = text[..start]
        .rfind(is_value_boundary)
        .map_or(0, |index| index + 1);
    let value_end = text[end..]
        .find(is_value_boundary)
        .map_or(text.len(), |index| end + index);
    let value = &text[value_start..value_end];
    if let Ok(url) = Url::parse(value)
        && url.has_host()
    {
        let relative_start = start - value_start;
        let relative_end = end - value_start;
        return relative_start >= url[..Position::BeforeHost].len()
            && relative_end <= url[..Position::AfterPort].len();
    }

    let relative_start = start - value_start;
    let relative_end = end - value_start;
    if !value[..relative_start].contains(['/', '\\'])
        && !value[relative_end..].contains(['/', '\\'])
    {
        return false;
    }

    let path = Path::new(value);
    path.has_root() || path.components().count() > 1 || value.contains('\\')
}

fn ignored_credential_match(
    provider: &providers::CredentialProvider,
    text: &str,
    start: usize,
    credential: &str,
) -> bool {
    provider
        .ignored_credential_prefixes
        .iter()
        .any(|prefix| credential.starts_with(prefix))
        && provider
            .credential_watermark
            .is_none_or(|watermark| !credential.contains(watermark))
        || credential.starts_with("sk-")
            && provider
                .credential_watermark
                .is_none_or(|watermark| !credential.contains(watermark))
            && credential[3..].split(['-', '_']).any(|segment| {
                segment.len() == 64
                    && segment.bytes().all(|byte| byte.is_ascii_hexdigit())
                    && credential
                        .find(segment)
                        .is_some_and(|offset| offset < provider.minimum_credential_len)
            })
            && text[..start]
                .rsplit(|character: char| {
                    character.is_ascii() && !character.is_ascii_alphanumeric()
                })
                .next()
                .is_some_and(|word| {
                    ((1..=3).contains(&word.len())
                        || word
                            .get(word.len().saturating_sub(2)..)
                            .is_some_and(|suffix| {
                                suffix.eq_ignore_ascii_case("di")
                                    || suffix.eq_ignore_ascii_case("ta")
                                        && !word.eq_ignore_ascii_case("data")
                            })
                        || credential.strip_prefix("sk-").is_some_and(|hash| {
                            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
                        }))
                        && word.bytes().all(|byte| byte.is_ascii_alphabetic())
                        && text[..start]
                            .rsplit_once(['/', '\\'])
                            .is_some_and(|(_, component)| {
                                component.chars().all(|character| {
                                    !character.is_ascii()
                                        || character.is_ascii_alphanumeric()
                                        || matches!(character, '_' | '-')
                                })
                            })
                })
}

pub(super) fn recognized_credential_match<'a>(
    provider: &providers::CredentialProvider,
    value: &'a str,
    virtualized: &str,
    start: usize,
) -> Option<&'a str> {
    let credential = builtin_credential_candidate(provider, value, start);
    let length = credential.len();
    let enclosing_start = value.as_bytes()[..start]
        .iter()
        .rposition(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-'))
        .map_or(0, |offset| offset + 1);
    let enclosing = &value[enclosing_start..start + length];
    (length >= provider.minimum_credential_len || !virtualized.contains(credential))
        .then_some(credential)
        .filter(|credential| !ignored_credential_match(provider, value, start, credential))
        .filter(|_| {
            !provider.ignored_credential_prefixes.iter().any(|prefix| {
                enclosing.match_indices(prefix).any(|(offset, _)| {
                    let ignored = &enclosing[offset..];
                    offset <= start - enclosing_start
                        && ignored_credential_match(
                            provider,
                            value,
                            enclosing_start + offset,
                            ignored,
                        )
                        && virtualized.contains(ignored)
                })
            })
        })
}

pub(super) fn builtin_credential_candidate<'a>(
    provider: &providers::CredentialProvider,
    value: &'a str,
    start: usize,
) -> &'a str {
    let mut length = value.as_bytes()[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    let candidate = &value[start..start + length];
    let current_prefix_len = provider
        .credential_prefixes
        .iter()
        .filter(|prefix| candidate.starts_with(**prefix))
        .map(|prefix| prefix.len())
        .max()
        .unwrap_or(0);
    if let Some(separator) = providers::credential_providers()
        .flat_map(|candidate_provider| {
            candidate_provider
                .credential_prefixes
                .iter()
                .map(move |prefix| (prefix, candidate_provider.minimum_credential_len))
        })
        .filter_map(|(candidate_prefix, minimum_length)| {
            candidate[current_prefix_len..]
                .match_indices(*candidate_prefix)
                .find_map(|(offset, _)| {
                    let offset = current_prefix_len + offset;
                    (matches!(candidate.as_bytes()[offset - 1], b'_' | b'-')
                        && offset > provider.minimum_credential_len
                        && candidate.len() - offset >= minimum_length)
                        .then_some(offset - 1)
                })
        })
        .min()
    {
        length = separator;
    }
    let candidate = &value[start..start + length];
    if value.as_bytes().get(start + length) == Some(&b'\0') {
        // A masked known span may follow an alias separator, not part of this builtin token.
        candidate.trim_end_matches(['_', '-'])
    } else {
        candidate
    }
}
