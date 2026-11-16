use super::*;
use pretty_assertions::assert_eq;
use std::time::Duration;

fn render(view: &FeedbackNoteView, width: u16) -> String {
    let height = view.desired_height(width);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    render_buffer(area, &buf)
}

fn render_buffer(area: Rect, buf: &Buffer) -> String {
    let mut lines: Vec<String> = (0..area.height)
        .map(|row| {
            let mut line = String::new();
            for col in 0..area.width {
                let symbol = buf[(area.x + col, area.y + row)].symbol();
                if symbol.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&crate::terminal_hyperlinks::strip_osc8(symbol));
                }
            }
            line.trim_end().to_string()
        })
        .collect();

    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn make_view(category: FeedbackCategory) -> FeedbackNoteView {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    FeedbackNoteView::new(
        category,
        /*turn_id*/ None,
        tx,
        /*include_logs*/ true,
        FeedbackAudience::External,
    )
}

#[test]
fn feedback_view_bad_result() {
    let view = make_view(FeedbackCategory::BadResult);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_bad_result", rendered);
}

#[test]
fn feedback_view_good_result() {
    let view = make_view(FeedbackCategory::GoodResult);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_good_result", rendered);
}

#[test]
fn feedback_view_bug() {
    let view = make_view(FeedbackCategory::Bug);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_bug", rendered);
}

#[test]
fn feedback_view_other() {
    let view = make_view(FeedbackCategory::Other);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_other", rendered);
}

#[test]
fn feedback_view_safety_check() {
    let view = make_view(FeedbackCategory::SafetyCheck);
    let rendered = render(&view, /*width*/ 60);
    insta::assert_snapshot!("feedback_view_safety_check", rendered);
}

#[test]
fn feedback_view_with_connectivity_diagnostics() {
    let (tx_raw, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let view = FeedbackNoteView::new(
        FeedbackCategory::Bug,
        /*turn_id*/ None,
        tx,
        /*include_logs*/ false,
        FeedbackAudience::External,
    );
    let rendered = render(&view, /*width*/ 60);

    insta::assert_snapshot!("feedback_view_with_connectivity_diagnostics", rendered);
}

#[test]
fn feedback_disclosure_matches_audience_at_normal_and_narrow_widths() {
    let mut rendered = Vec::new();
    for (audience, name, url, link_label) in [
        (
            FeedbackAudience::External,
            "external",
            FEEDBACK_POLICY_URL,
            "Learnmore",
        ),
        (
            FeedbackAudience::OpenAiEmployee,
            "employee",
            EMPLOYEE_FEEDBACK_FAQ_URL,
            "here",
        ),
    ] {
        let mut view = make_view(FeedbackCategory::Bug);
        view.feedback_audience = audience;
        for width in [80, 40] {
            let area = Rect::new(
                /*x*/ 0,
                /*y*/ 0,
                width,
                view.desired_height(width),
            );
            let mut buf = Buffer::empty(area);
            view.render(area, &mut buf);

            let linked_text = buf
                .content
                .iter()
                .filter(|cell| cell.symbol().contains(&format!("\x1b]8;;{url}\x07")))
                .map(|cell| crate::terminal_hyperlinks::strip_osc8(cell.symbol()))
                .collect::<String>();
            assert_eq!(linked_text, link_label);
            rendered.push(format!(
                "{name} ({width} columns)\n{}",
                render_buffer(area, &buf)
            ));
        }
    }
    insta::assert_snapshot!("feedback_disclosures", rendered.join("\n\n"));
}

#[test]
fn feedback_disclosure_scrolls_without_hiding_input_in_short_panes() {
    let mut rendered = Vec::new();
    for (width, height) in [(40, 12), (80, 8)] {
        let mut view = make_view(FeedbackCategory::Bug);
        view.feedback_audience = FeedbackAudience::OpenAiEmployee;
        view.handle_paste("A visible note.".to_string());
        let area = Rect::new(/*x*/ 0, /*y*/ 0, width, height);
        for (stage, key) in [
            ("initial", None),
            ("scrolled", Some(KeyCode::PageDown)),
            ("restored", Some(KeyCode::PageUp)),
        ] {
            if let Some(key) = key {
                view.handle_key_event(KeyEvent::from(key));
            }
            let mut buf = Buffer::empty(area);
            view.render(area, &mut buf);
            let text = render_buffer(area, &buf);
            assert!(text.contains("A visible note."));
            assert!(text.contains("enter to submit"));
            assert!(
                view.cursor_pos(area)
                    .is_some_and(|(x, y)| x < width && y < height)
            );
            assert_eq!(view.is_complete(), false);
            if stage == "scrolled" {
                assert!(text.contains("sensitive personal"));
            }
            rendered.push(format!("{width}x{height} {stage}\n{text}"));
        }
    }
    insta::assert_snapshot!("feedback_disclosure_short_panes", rendered.join("\n\n"));
}

#[test]
fn feedback_disclosure_keeps_editor_visible_in_four_rows() {
    let mut view = make_view(FeedbackCategory::Bug);
    view.feedback_audience = FeedbackAudience::OpenAiEmployee;
    view.handle_paste("A visible note.".to_string());
    let area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 80, /*height*/ 4,
    );
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);

    assert!(
        view.cursor_pos(area)
            .is_some_and(|(x, y)| area.contains((x, y).into()))
    );
    let initial = render_buffer(area, &buf);
    assert!(initial.contains("Your data may be used to improve our models and products"));
    for _ in view.intro_lines(area.width, IntroContent::Full) {
        view.handle_key_event(KeyEvent::from(KeyCode::PageDown));
        buf = Buffer::empty(area);
        view.render(area, &mut buf);
    }
    let scrolled = render_buffer(area, &buf);
    assert!(scrolled.contains("personal information."));
    insta::assert_snapshot!(
        "feedback_disclosure_four_rows",
        format!("{initial}\n\n{scrolled}")
    );

    let narrow_area = Rect::new(
        /*x*/ 0, /*y*/ 0, /*width*/ 24, /*height*/ 4,
    );
    for (audience, disclosure) in [
        (FeedbackAudience::OpenAiEmployee, "Your data"),
        (FeedbackAudience::External, "Your feedback"),
    ] {
        let mut view = make_view(FeedbackCategory::SafetyCheck);
        view.feedback_audience = audience;
        let mut buf = Buffer::empty(narrow_area);
        view.render(narrow_area, &mut buf);
        assert!(render_buffer(narrow_area, &buf).contains(disclosure));
    }
}

#[test]
fn feedback_paste_burst_preserves_multiline_note_until_submit() {
    for include_logs in [false, true] {
        for note in ["x\nrest", "id\n\nbody", "foo\nbar", "x\t\nrest"] {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let mut view = FeedbackNoteView::new(
                FeedbackCategory::Bug,
                /*turn_id*/ None,
                AppEventSender::new(tx),
                include_logs,
                FeedbackAudience::External,
            );
            let now = Instant::now();
            for (index, ch) in note.chars().enumerate() {
                let code = match ch {
                    '\n' => KeyCode::Enter,
                    '\t' => KeyCode::Tab,
                    ch => KeyCode::Char(ch),
                };
                view.handle_key_event_at(
                    KeyEvent::from(code),
                    now + Duration::from_millis(index as u64),
                );
                assert!(!view.is_complete());
                assert!(rx.try_recv().is_err());
            }
            let expected_note = note.replace('\t', "");
            assert_eq!(view.textarea.text(), expected_note);
            if !include_logs && note == "foo\nbar" {
                insta::assert_snapshot!("feedback_multiline_paste", render(&view, /*width*/ 60));
            }

            view.handle_key_event_at(
                KeyEvent::from(KeyCode::Enter),
                now + Duration::from_millis(200),
            );

            let AppEvent::SubmitFeedback {
                category,
                reason,
                report_passcode,
                turn_id,
                include_logs: submitted_logs,
            } = rx.try_recv().expect("submit feedback event")
            else {
                panic!("expected feedback submission");
            };
            assert_eq!(
                (category, reason, turn_id, submitted_logs),
                (
                    FeedbackCategory::Bug,
                    Some(expected_note),
                    None,
                    include_logs
                )
            );
            assert!(report_passcode.is_none());
            assert!(view.is_complete());
            assert!(rx.try_recv().is_err());
        }
    }
}

#[test]
fn feedback_bracketed_paste_does_not_reach_chat_composer() {
    for include_logs in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let tx = AppEventSender::new(tx);
        let mut pane = super::super::tests::test_pane(tx.clone());
        pane.show_view(Box::new(FeedbackNoteView::new(
            FeedbackCategory::Bug,
            /*turn_id*/ None,
            tx,
            include_logs,
            FeedbackAudience::External,
        )));
        // An explicit paste ends any preceding keystroke burst.
        pane.handle_key_event(KeyEvent::from(KeyCode::Char('x')));
        pane.handle_paste("\nrest".to_string());
        assert!(rx.try_recv().is_err());
        assert!(matches!(
            pane.handle_key_event(KeyEvent::from(KeyCode::Enter)),
            super::super::InputResult::None
        ));
        let AppEvent::SubmitFeedback {
            reason,
            include_logs: submitted_logs,
            ..
        } = rx.try_recv().expect("submit feedback event")
        else {
            panic!("expected feedback submission");
        };
        assert_eq!(
            (reason, submitted_logs),
            (Some("x\nrest".into()), include_logs)
        );
        assert!(pane.composer_is_empty());
        assert!(rx.try_recv().is_err());
    }
}

#[test]
fn submit_feedback_emits_submit_event_with_trimmed_note() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut view = FeedbackNoteView::new(
        FeedbackCategory::Bug,
        Some("turn-123".to_string()),
        tx,
        /*include_logs*/ true,
        FeedbackAudience::OpenAiEmployee,
    );
    view.textarea.insert_str("  something broke  ");

    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

    let event = rx.try_recv().expect("submit feedback event");
    assert!(matches!(
        event,
        AppEvent::SubmitFeedback {
            category: FeedbackCategory::Bug,
            reason: Some(reason),
            report_passcode: None,
            turn_id: Some(turn_id),
            include_logs: true,
        } if reason == "something broke" && turn_id == "turn-123"
    ));
    assert_eq!(view.is_complete(), true);
}

#[test]
fn submit_feedback_omits_empty_note() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let tx = AppEventSender::new(tx_raw);
    let mut view = FeedbackNoteView::new(
        FeedbackCategory::GoodResult,
        /*turn_id*/ None,
        tx,
        /*include_logs*/ false,
        FeedbackAudience::External,
    );

    view.submit();

    let event = rx.try_recv().expect("submit feedback event");
    assert!(matches!(
        event,
        AppEvent::SubmitFeedback {
            category: FeedbackCategory::GoodResult,
            reason: None,
            report_passcode: None,
            turn_id: None,
            include_logs: false,
        }
    ));
}

#[test]
fn dedicated_passcode_is_masked_separate_and_required_for_opt_in() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let mut view = FeedbackNoteView::new_with_passcode_requirement(
        FeedbackCategory::Bug,
        None,
        AppEventSender::new(tx_raw),
        false,
        FeedbackAudience::External,
        true,
    );
    view.textarea.insert_str("ordinary report note");
    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err());
    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(rx.try_recv().is_err(), "missing passcode cannot submit");
    assert!(view.handle_paste("invalid\noperator-secret-123".into()));
    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(
        rx.try_recv().is_err(),
        "control characters cannot be pasted into passcodes"
    );
    assert!(view.handle_paste("operator-secret-123".into()));
    let area = Rect::new(0, 0, 70, 4);
    let mut buf = Buffer::empty(area);
    view.render(area, &mut buf);
    assert!(!render_buffer(area, &buf).contains("operator-secret-123"));
    view.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let event = rx.try_recv().unwrap();
    let debug = format!("{event:?}");
    assert!(!debug.contains("operator-secret-123"));
    match event {
        AppEvent::SubmitFeedback {
            reason,
            report_passcode,
            ..
        } => {
            assert_eq!(reason.as_deref(), Some("ordinary report note"));
            assert_eq!(report_passcode.unwrap().into_inner(), "operator-secret-123");
        }
        _ => panic!("expected feedback submission"),
    }
}

#[test]
fn dedicated_passcode_waits_for_multiline_note_paste_to_end() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let mut view = FeedbackNoteView::new_with_passcode_requirement(
        FeedbackCategory::Bug,
        None,
        AppEventSender::new(tx_raw),
        false,
        FeedbackAudience::External,
        true,
    );
    let start = Instant::now();
    for (index, ch) in "line one\nline two".chars().enumerate() {
        view.handle_key_event_at(
            KeyEvent::from(if ch == '\n' {
                KeyCode::Enter
            } else {
                KeyCode::Char(ch)
            }),
            start + Duration::from_millis(index as u64),
        );
    }
    assert_eq!(view.textarea.text(), "line one\nline two");
    assert!(!view.entering_passcode);
    assert!(rx.try_recv().is_err());
    view.handle_key_event_at(
        KeyEvent::from(KeyCode::Enter),
        start + Duration::from_millis(200),
    );
    assert!(view.entering_passcode);
    assert!(rx.try_recv().is_err());
    assert!(view.handle_paste("operator-secret-123".into()));
    view.handle_key_event(KeyEvent::from(KeyCode::Enter));
    let AppEvent::SubmitFeedback {
        reason,
        report_passcode,
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("expected feedback submission");
    };
    assert_eq!(reason.as_deref(), Some("line one\nline two"));
    assert_eq!(report_passcode.unwrap().into_inner(), "operator-secret-123");
}

#[test]
fn server_requested_passcode_retries_original_note_without_putting_secret_in_reason() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let mut view = FeedbackNoteView::passcode_retry(
        FeedbackCategory::Bug,
        Some("turn-123".into()),
        AppEventSender::new(tx_raw),
        false,
        FeedbackAudience::External,
        Some("original report note".into()),
    );
    assert!(view.entering_passcode);
    assert_eq!(view.textarea.text(), "original report note");
    assert!(rx.try_recv().is_err());
    assert!(view.handle_paste("operator-secret-123".into()));
    assert!(!render(&view, 70).contains("operator-secret-123"));
    view.handle_key_event(KeyEvent::from(KeyCode::Enter));
    let AppEvent::SubmitFeedback {
        reason,
        report_passcode,
        turn_id,
        ..
    } = rx.try_recv().unwrap()
    else {
        panic!("expected feedback retry");
    };
    assert_eq!(reason.as_deref(), Some("original report note"));
    assert_eq!(turn_id.as_deref(), Some("turn-123"));
    assert_eq!(report_passcode.unwrap().into_inner(), "operator-secret-123");
}

#[test]
fn feedback_note_cancel_does_not_submit() {
    let (tx_raw, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    let mut view = FeedbackNoteView::new(
        FeedbackCategory::Bug,
        /*turn_id*/ None,
        AppEventSender::new(tx_raw),
        /*include_logs*/ false,
        FeedbackAudience::OpenAiEmployee,
    );
    view.textarea.insert_str("unfinished feedback");

    view.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert_eq!(view.is_complete(), true);
    assert!(rx.try_recv().is_err());
}
