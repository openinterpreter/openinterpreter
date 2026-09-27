use super::CredentialBrokerState;
use super::CredentialRecord;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::ops::Range;

impl CredentialBrokerState {
    pub(super) fn replace_child_env_dummies(
        &self,
        env: &mut HashMap<String, String>,
        replacements: &[(String, String)],
    ) {
        if replacements.is_empty() {
            return;
        }
        let credentials = self
            .credentials
            .iter()
            .map(|credential| {
                let replacement = replacements
                    .iter()
                    .find(|(dummy, _)| *dummy == credential.dummy_value)
                    .map_or(credential.dummy_value.as_str(), |(_, replacement)| {
                        replacement
                    });
                let target = self
                    .credentials
                    .iter()
                    .find(|candidate| candidate.dummy_value == replacement);
                (credential, replacement, target)
            })
            .collect::<Vec<_>>();
        for value in env.values_mut() {
            let mut spans = Replacements::default();
            for (credential, replacement, target) in &credentials {
                spans.add(
                    credential.value_match_ranges(value, &credential.dummy_value),
                    replacement,
                    *target,
                );
            }
            spans.render(value);
        }
    }
}

// Retain emitted context without keeping another copy of any real credentials in it.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct GeneratedAlias {
    digest: [u8; 32],
    len: usize,
    range: Range<usize>,
}

impl CredentialRecord {
    pub(super) fn generated_dummy_ranges(&self, text: &str) -> Vec<Range<usize>> {
        let aliases = self
            .generated_aliases
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        text.char_indices()
            .filter(|(start, _)| text[*start..].starts_with(&self.dummy_value))
            .filter_map(|(start, _)| {
                aliases
                    .iter()
                    .any(|alias| {
                        start.checked_sub(alias.range.start).is_some_and(|offset| {
                            text.get(offset..offset + alias.len).is_some_and(|context| {
                                <[u8; 32]>::from(Sha256::digest(context)) == alias.digest
                            })
                        })
                    })
                    .then_some(start..start + self.dummy_value.len())
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct Replacements<'a> {
    spans: Vec<(Range<usize>, &'a str, Option<&'a CredentialRecord>)>,
}

impl<'a> Replacements<'a> {
    pub(super) fn add(
        &mut self,
        ranges: impl IntoIterator<Item = Range<usize>>,
        value: &'a str,
        dummy: Option<&'a CredentialRecord>,
    ) {
        self.spans
            .extend(ranges.into_iter().map(|range| (range, value, dummy)));
    }

    pub(super) fn masked(&self, text: &str) -> String {
        let mut masked = text.to_string();
        for (range, _, _) in &self.spans {
            masked.replace_range(range.clone(), &"\0".repeat(range.len()));
        }
        masked
    }

    pub(super) fn render(mut self, text: &mut String) -> bool {
        if self.spans.is_empty() {
            return false;
        }
        self.spans
            .sort_by_key(|(range, _, _)| std::cmp::Reverse(range.len()));
        let mut selected = Vec::<(Range<usize>, &str, Option<&CredentialRecord>)>::new();
        for span in self.spans {
            if !selected
                .iter()
                .any(|(range, _, _)| range.start < span.0.end && span.0.start < range.end)
            {
                selected.push(span);
            }
        }
        selected.sort_by_key(|(range, _, _)| range.start);
        let mut output = String::with_capacity(text.len());
        let mut previous = 0;
        let mut dummies = Vec::new();
        for (range, value, dummy) in selected {
            output.push_str(&text[previous..range.start]);
            let start = output.len();
            output.push_str(value);
            if let Some(dummy) = dummy {
                dummies.push((dummy, start..output.len()));
            }
            previous = range.end;
        }
        output.push_str(&text[previous..]);
        for (dummy, range) in dummies {
            if !dummy
                .value_match_ranges(&output, &dummy.dummy_value)
                .contains(&range)
            {
                let alias = GeneratedAlias {
                    digest: Sha256::digest(&output).into(),
                    len: output.len(),
                    range,
                };
                let mut aliases = dummy
                    .generated_aliases
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if !aliases.contains(&alias) {
                    aliases.push(alias);
                }
            }
        }
        *text = output;
        true
    }
}
