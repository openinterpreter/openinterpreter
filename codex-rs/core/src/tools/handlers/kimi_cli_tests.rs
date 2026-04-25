use super::KimiEdit;
use super::apply_kimi_edit;
use super::format_kimi_read_output;
use pretty_assertions::assert_eq;

#[test]
fn format_kimi_read_output_numbers_lines() {
    let output = format_kimi_read_output("alpha\nbeta\ngamma\n", 2, 2);
    assert_eq!(output.body, "     2\tbeta\n     3\tgamma\n");
    assert_eq!(
        output.system_message,
        "<system>2 lines read from file starting from line 2. Total lines in file: 3. End of file reached.</system>"
    );
}

#[test]
fn format_kimi_read_output_supports_negative_offsets() {
    let output = format_kimi_read_output("alpha\nbeta\ngamma\n", -2, 2);
    assert_eq!(output.body, "     2\tbeta\n     3\tgamma\n");
    assert_eq!(
        output.system_message,
        "<system>2 lines read from file starting from line 2. Total lines in file: 3. End of file reached.</system>"
    );
}

#[test]
fn apply_kimi_edit_replaces_first_match_by_default() {
    let (output, replacement_count) = apply_kimi_edit(
        "one two one",
        &KimiEdit {
            old: "one".to_string(),
            new: "ONE".to_string(),
            replace_all: None,
        },
    );
    assert_eq!(output, "ONE two one");
    assert_eq!(replacement_count, 1);
}

#[test]
fn apply_kimi_edit_replaces_all_matches_when_requested() {
    let (output, replacement_count) = apply_kimi_edit(
        "one two one",
        &KimiEdit {
            old: "one".to_string(),
            new: "ONE".to_string(),
            replace_all: Some(true),
        },
    );
    assert_eq!(output, "ONE two ONE");
    assert_eq!(replacement_count, 2);
}
