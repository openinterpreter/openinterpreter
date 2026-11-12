//! Coverage for name-first row retention and whole-character description budgets.

use super::calculate_namespace_row_truncation;
use super::truncate_namespace_rows;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn truncates_raw_rows_at_character_boundaries_in_group_order() {
    let first = BTreeMap::from([
        ("🦀".to_string(), "&🦀x".to_string()),
        ("&".to_string(), "🦀&y".to_string()),
    ]);
    let empty = BTreeMap::new();
    let last = BTreeMap::from([("b".to_string(), "x<&".to_string())]);
    let groups = [("first", &first), ("empty", &empty), ("last", &last)];
    let cases = [
        (
            36,
            64,
            [
                Some("- &: 🦀&y\n"),
                Some("- 🦀: &🦀x\n"),
                Some("- b: x<&\n"),
            ],
        ),
        (
            32,
            64,
            [Some("- &: ...\n"), Some("- 🦀: &...\n"), Some("- b\n")],
        ),
        (15, 64, [Some("- &\n"), Some("- 🦀\n"), Some("- b\n")]),
        (14, 6, [Some("- &\n"), None, Some("- b\n")]),
    ];

    for (byte_budget, omission_reserve_bytes, expected) in cases {
        assert_eq!(
            truncate_namespace_rows(&groups, byte_budget, omission_reserve_bytes),
            expected
                .into_iter()
                .map(|row| row.map(str::to_owned))
                .collect::<Vec<_>>(),
            "byte budget: {byte_budget}, omission reserve bytes: {omission_reserve_bytes}"
        );
    }
}

#[test]
fn marks_truncated_descriptions_within_the_allocated_bytes() {
    let cases = [
        ("abcdef", 4, "- n\n"),
        ("abcdef", 7, "- n\n"),
        ("abcdef", 8, "- n\n"),
        ("abcdef", 9, "- n: ...\n"),
        ("abcdef", 10, "- n: a...\n"),
        ("abcdef", 12, "- n: abcdef\n"),
        ("ab", 8, "- n: ab\n"),
        ("🦀abcdef", 10, "- n: ...\n"),
        ("🦀abcdef", 13, "- n: 🦀...\n"),
        ("éabcdef", 11, "- n: é...\n"),
    ];

    for (description, byte_budget, expected) in cases {
        let namespaces = BTreeMap::from([("n".to_string(), description.to_string())]);
        let rows = truncate_namespace_rows(
            &[("namespaces", &namespaces)],
            byte_budget,
            /*omission_reserve_bytes*/ 64,
        );
        assert_eq!(
            rows,
            vec![Some(expected.to_string())],
            "description: {description:?}, byte budget: {byte_budget}"
        );
        assert!(rows.iter().flatten().map(String::len).sum::<usize>() <= byte_budget);
    }
}

#[test]
fn allocates_complete_description_prefixes_in_stable_round_robin_order() {
    // Budgets include separators; retained byte offsets refer only to descriptions.
    let cases = [
        (vec![], 9, vec![]),
        (vec!["", ""], 9, vec![0, 0]),
        (vec!["", "ab", ""], 4, vec![0, 2, 0]),
        (vec!["a", "🦀"], 0, vec![0, 0]),
        (vec!["a", "🦀"], 2, vec![0, 0]),
        (vec!["é€🦀", "xy"], 20, vec![9, 2]),
        (vec!["é€🦀", "xy"], 12, vec![5, 2]),
        (vec!["ab", "cd"], 7, vec![2, 1]),
        (vec!["a", "bcdef"], 9, vec![1, 4]),
        (vec!["a🦀x", "bcdef"], 9, vec![1, 4]),
        (vec!["🦀x", "abc"], 5, vec![0, 3]),
        (vec!["🦀🦀x", "€🦀y"], 18, vec![9, 3]),
        (vec!["🦀🦀x", "€🦀y"], 19, vec![8, 7]),
    ];

    for (descriptions, description_byte_budget, expected) in cases {
        let entries = descriptions
            .iter()
            .map(|description| ("n", *description))
            .collect::<Vec<_>>();
        assert_eq!(
            calculate_namespace_row_truncation(
                &entries,
                description_byte_budget + 4 * entries.len(),
                /*omission_reserve_bytes*/ 64
            ),
            expected.into_iter().map(Some).collect::<Vec<_>>(),
            "descriptions: {descriptions:?}, description byte budget: {description_byte_budget}"
        );
    }
}

#[test]
fn retains_names_before_descriptions_and_reserves_omissions_only_on_overflow() {
    let cases = [
        // Every name is retained before any description consumes the budget.
        (
            vec![("a", "abcd"), ("b", "x")],
            11,
            64,
            vec![Some(1), Some(0)],
        ),
        // Names alone exactly fit, so the omission reserve is unnecessary.
        (vec![("a", ""), ("b", "")], 8, 64, vec![Some(0), Some(0)]),
        // Charging the reserve skips the second name; spare bytes stay unused.
        (
            vec![("aaaa", "x"), ("bbbb", "x"), ("c", "x")],
            15,
            3,
            vec![Some(0), None, Some(0)],
        ),
        // An oversized name cannot prevent later complete names from fitting.
        (
            vec![("oversized_name_aaa", "x"), ("a", "x"), ("b", "x")],
            14,
            2,
            vec![None, Some(0), Some(0)],
        ),
        (vec![("ab", "x"), ("c", "")], 0, 2, vec![None, None]),
        (vec![("a", "x"), ("b", "x")], 6, 64, vec![None, None]),
    ];

    for (entries, byte_budget, omission_reserve_bytes, expected) in cases {
        assert_eq!(
            calculate_namespace_row_truncation(&entries, byte_budget, omission_reserve_bytes),
            expected,
            "entries: {entries:?}, byte budget: {byte_budget}, omission reserve bytes: {omission_reserve_bytes}"
        );
    }
}
