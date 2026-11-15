#![cfg(test)]

use super::StatusLine;
use super::StatusLineFocus;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::osc8_hyperlink;
use pretty_assertions::assert_eq;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::layout::Size;

#[test]
fn extension_layout_cache_is_bound_to_its_content_and_width() {
    use crate::ui_tree::UiTree;

    let first: UiTree = r#"{"type":"text","children":["short"]}"#.parse().unwrap();
    let second: UiTree = r#"{"type":"text","children":["longer replacement"]}"#
        .parse()
        .unwrap();
    let first = StatusLine::from(first);
    let second = StatusLine::from(second);
    let bounds = Size::new(8, 4);
    assert_eq!(first.height(bounds), 1);
    assert!(second.height(bounds) > first.height(bounds));
    assert_eq!(second.height(Size::new(32, 4)), 1);

    let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 4));
    second.render(Rect::new(0, 0, 8, 4), &mut buffer, StatusLineFocus::Focused);
    let painted: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
    assert!(painted.contains("longer"), "{painted:?}");
    assert!(!painted.contains("short"), "{painted:?}");
}

#[test]
fn clipped_links_keep_their_cells_after_a_halfwidth_katakana_grapheme() {
    let project_url = "https://example.com/project";
    let pr_url = "https://example.com/pr/42";
    let mut line = HyperlinkLine::default();
    line.push_span("ｶﾞ".into(), Some(project_url));
    line.push_span(" · ".into(), /*destination*/ None);
    line.push_span("PR #42".into(), Some(pr_url));

    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 10, /*height*/ 1,
    );
    let mut buf = Buffer::empty(area);
    StatusLine::from(line).render(area, &mut buf, StatusLineFocus::Focused);
    assert_eq!(buf[(0, 0)].symbol(), osc8_hyperlink(project_url, "ｶﾞ"));
    for (x, ch) in (2..5).zip(" · ".chars()) {
        assert_eq!(buf[(x, 0)].symbol(), ch.to_string());
    }
    for (x, ch) in (5..9).zip("PR #".chars()) {
        assert_eq!(
            buf[(x, 0)].symbol(),
            osc8_hyperlink(pr_url, &ch.to_string())
        );
    }
    assert_eq!(buf[(9, 0)].symbol(), "…");
}

#[test]
fn arabic_prefix_keeps_link_on_the_painted_cells_at_exact_fit_and_overflow() {
    let url = "https://example.com/pr/42";
    let mut line = HyperlinkLine::from("لا · ");
    line.push_span("PR #42".into(), Some(url));
    assert_eq!(line.width(), 11);
    let content = StatusLine::from(line);

    for (width, visible_link) in [(11, "PR #42"), (10, "PR #")] {
        let area = Rect::new(/*x*/ 0, /*y*/ 0, width, /*height*/ 1);
        let mut buf = Buffer::empty(area);
        content.render(area, &mut buf, StatusLineFocus::Focused);
        assert_eq!(buf[(4, 0)].symbol(), " ");
        for (x, ch) in (5..width).zip(visible_link.chars()) {
            assert_eq!(buf[(x, 0)].symbol(), osc8_hyperlink(url, &ch.to_string()));
        }
        if width < 11 {
            assert_eq!(buf[(width - 1, 0)].symbol(), "…");
        }
    }
}

#[test]
fn prepared_tree_paints_only_the_footer_viewport() {
    use crate::terminal_hyperlinks::strip_osc8;
    use crate::ui_tree::UiTree;
    use ratatui::buffer::Cell;

    let url = "https://example.com/build";
    let tree: UiTree = serde_json::json!({
        "type":"link","props":{"href":url},"children":["ok\nlarge build\na\nb\nc\nd"]
    })
    .to_string()
    .parse()
    .unwrap();
    let prepared = StatusLine::from(tree);

    let area = Rect::new(
        /*x*/ 1, /*y*/ 1, /*width*/ 6, /*height*/ 4,
    );
    let mut buffer = Buffer::filled(
        Rect::new(
            /*x*/ 0, /*y*/ 0, /*width*/ 8, /*height*/ 7,
        ),
        Cell::new("."),
    );
    prepared.render(area, &mut buffer, StatusLineFocus::Focused);
    let rows: Vec<String> = buffer
        .content
        .chunks(8)
        .map(|row| row.iter().map(|cell| strip_osc8(cell.symbol())).collect())
        .collect();
    insta::assert_snapshot!(rows.join("\n"));

    assert_eq!(buffer[(1, 4)].symbol(), osc8_hyperlink(url, "a"));
    assert_eq!(buffer[(2, 4)].symbol(), "…");
}
