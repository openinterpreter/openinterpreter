use super::wrap_hint_rows;
use pretty_assertions::assert_eq;
use unicode_width::UnicodeWidthStr;

#[test]
fn wraps_whole_hints_using_display_width() {
    let hints = ["↑ 上", "↓ 下", "enter select"];
    assert_eq!(
        wrap_hint_rows(
            hints,
            /*width*/ 9,
            /*separator_width*/ 1,
            |hint| { UnicodeWidthStr::width(*hint) }
        ),
        vec![vec!["↑ 上", "↓ 下"], vec!["enter select"]]
    );
}

#[test]
fn retains_oversized_hints_and_an_empty_row() {
    assert_eq!(
        wrap_hint_rows(
            ["ctrl+x stop", "esc back"],
            /*width*/ 0,
            /*separator_width*/ 3,
            |hint| hint.len()
        ),
        vec![vec!["ctrl+x stop"], vec!["esc back"]]
    );
    assert_eq!(
        wrap_hint_rows(
            Vec::<String>::new(),
            /*width*/ 40,
            /*separator_width*/ 3,
            String::len
        ),
        vec![Vec::<String>::new()]
    );
}
