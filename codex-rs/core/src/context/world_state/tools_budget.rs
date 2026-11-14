//! Truncates borrowed namespace text within a shared byte budget.
//! Character iterators preserve UTF-8 boundaries and stable input order without
//! per-character tables. Callers account for headings and format omission notices.

use std::collections::BTreeMap;
use std::collections::VecDeque;

pub(super) const DESCRIPTION_TRUNCATION_SUFFIX: &str = "...";

/// Returns complete rows in group order, with `None` for omitted names.
/// The budget excludes headings; omission space is charged only when names overflow.
pub(super) fn truncate_namespace_rows(
    groups: &[(&str, &BTreeMap<String, String>)],
    byte_budget: usize,
    omission_reserve_bytes: usize,
) -> Vec<Option<String>> {
    let entries = groups
        .iter()
        .flat_map(|(_, namespaces)| namespaces.iter())
        .map(|(namespace, description)| (namespace.as_str(), description.as_str()))
        .collect::<Vec<_>>();
    let retained_description_bytes =
        calculate_namespace_row_truncation(&entries, byte_budget, omission_reserve_bytes);
    entries
        .into_iter()
        .zip(retained_description_bytes)
        .map(|((namespace, description), retained_description_bytes)| {
            retained_description_bytes.map(|retained_description_bytes| {
                if retained_description_bytes == 0 {
                    format!("- {namespace}\n")
                } else if retained_description_bytes == description.len() {
                    format!("- {namespace}: {description}\n")
                } else if let Some(prefix_bytes) =
                    retained_description_bytes.checked_sub(DESCRIPTION_TRUNCATION_SUFFIX.len())
                {
                    let prefix = &description[..description.floor_char_boundary(prefix_bytes)];
                    format!("- {namespace}: {prefix}{DESCRIPTION_TRUNCATION_SUFFIX}\n")
                } else {
                    format!("- {namespace}\n")
                }
            })
        })
        .collect()
}

/// Reserves all names before sharing description space in character-round order.
/// `None` omits a row; `Some(0)` keeps its name; `Some(n)` keeps n description bytes.
fn calculate_namespace_row_truncation(
    entries: &[(&str, &str)],
    byte_budget: usize,
    omission_reserve_bytes: usize,
) -> Vec<Option<usize>> {
    let total_name_bytes = entries
        .iter()
        .map(|(namespace, _)| "- ".len() + namespace.len() + "\n".len())
        .sum::<usize>();
    if total_name_bytes > byte_budget {
        let mut remaining_bytes = byte_budget.saturating_sub(omission_reserve_bytes);
        return entries
            .iter()
            .map(|(namespace, _)| {
                let name_only_bytes = "- ".len() + namespace.len() + "\n".len();
                if name_only_bytes <= remaining_bytes {
                    remaining_bytes -= name_only_bytes;
                    Some(0)
                } else {
                    None
                }
            })
            .collect();
    }

    let mut remaining_bytes = byte_budget - total_name_bytes;
    let total_description_bytes = entries
        .iter()
        .filter(|(_, description)| !description.is_empty())
        .map(|(_, description)| ": ".len() + description.len())
        .sum::<usize>();
    if total_description_bytes <= remaining_bytes {
        return entries
            .iter()
            .map(|(_, description)| Some(description.len()))
            .collect();
    }

    let mut retained_description_bytes = vec![Some(0); entries.len()];
    if remaining_bytes == 0 {
        return retained_description_bytes;
    }
    let mut active_rows = VecDeque::with_capacity(entries.len());
    active_rows.extend(
        entries
            .iter()
            .enumerate()
            .filter(|(_, (_, description))| !description.is_empty())
            .map(|(row_index, (_, description))| (row_index, description.char_indices())),
    );
    // Each visit considers one character. Exhausted rows and prefixes that cannot
    // fit leave the queue permanently, because the remaining budget only shrinks.
    while let Some((row_index, mut characters)) = active_rows.pop_front() {
        let Some((byte_index, ch)) = characters.next() else {
            continue;
        };
        let character_bytes = ch.len_utf8();
        let extra_bytes = character_bytes + if byte_index == 0 { ": ".len() } else { 0 };
        if extra_bytes <= remaining_bytes {
            remaining_bytes -= extra_bytes;
            retained_description_bytes[row_index] = Some(byte_index + character_bytes);
            active_rows.push_back((row_index, characters));
        }
    }
    retained_description_bytes
}

#[cfg(test)]
#[path = "tools_budget_tests.rs"]
mod tests;
