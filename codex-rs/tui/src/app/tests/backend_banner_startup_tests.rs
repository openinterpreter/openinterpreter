use super::backend_banner_fallback_tests::fallback_response;
use super::*;
use crate::chatwidget::UserMessage;
use crate::chatwidget::tests::helpers::set_fast_mode_test_catalog_for_models;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn backend_banner_fallback_applies_before_initial_and_queued_startup_prompts() -> Result<()> {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let mut config = app.config.clone();
    config.model = Some("gpt-5.5".into());
    let mut tui = crate::tui::test_support::make_test_tui()?;
    let init = app.chatwidget_init_for_forked_or_resumed_thread(
        &mut tui,
        config.clone(),
        Some(UserMessage::from("initial prompt")),
    );
    app.replace_chat_widget(ChatWidget::new_with_app_event(init));
    set_chatgpt_auth(&mut app.chat_widget);
    set_fast_mode_test_catalog_for_models(&mut app.chat_widget, "gpt-5.5", "gpt-5.6-terra");
    app.model_catalog = app.chat_widget.model_catalog();
    app.pending_startup_thread_start = true;
    app.chat_widget.set_model("gpt-5.5");
    app.chat_widget
        .set_queue_submissions_until_session_configured(/*queue*/ true);
    app.chat_widget
        .apply_external_edit("queued follow-up".into());
    app.chat_widget
        .handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let (mut server, requests, proxy) = session_lifecycle_requests::start_recording_app_server(
        &config, /*blocked_thread_list*/ None, /*failed_thread_name*/ None,
    )
    .await?;
    let generation = app.rate_limit_hard_stop_generation;
    app.handle_event(
        &mut tui,
        &mut server,
        AppEvent::RateLimitsLoaded {
            request_id: 1,
            origin: RateLimitRefreshOrigin::StartupPrefetch {
                reset_hint_request_id: 0,
            },
            hard_stop_generation: generation,
            result: Ok(fallback_response()),
        },
    )
    .await?;
    assert!(requests.lock().unwrap().is_empty());
    assert!(
        !render_bottom_popup(&app.chat_widget, /*width*/ 72)
            .contains("Selected model usage exhausted")
    );
    let started = server.start_thread(&config).await?;
    let thread_id = started.session.thread_id;
    requests.lock().unwrap().clear();
    while events.try_recv().is_ok() {}
    app.handle_startup_thread_started(&mut server, Ok(started))
        .await?;
    assert_eq!(app.active_thread_id, Some(thread_id));
    let submissions = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| {
            if let AppEvent::CodexOp(AppCommand::UserTurn { model, items, .. }) = event {
                Some((model, items))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(
        submissions,
        vec![(
            "gpt-5.6-terra".into(),
            vec![UserInput::Text {
                text: "initial prompt".into(),
                text_elements: Vec::new(),
            }]
        )]
    );
    assert_eq!(
        app.chat_widget.queued_user_message_texts(),
        vec!["queued follow-up"]
    );
    assert_eq!(
        requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.method.as_str())
            .collect::<Vec<_>>(),
        vec!["thread/settings/update"]
    );
    server.shutdown().await?;
    proxy.await??;
    Ok(())
}
