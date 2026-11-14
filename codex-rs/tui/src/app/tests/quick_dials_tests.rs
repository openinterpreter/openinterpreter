//! Preset persistence tests for file writes, layer ownership, and failure atomicity.

use super::*;
use codex_config::LoaderOverrides;
use codex_config::types::QuickDial;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn quick_dials_persist_save_replace_and_delete() -> Result<()> {
    let (mut app, _events, _ops) = make_test_app_with_channels().await;
    let config_path = app.local_settings.user_config_path.clone();
    std::fs::write(
        config_path.as_path(),
        "# Keep this setting.\n[tui]\nshow_tooltips = false\n",
    )?;

    let first = QuickDial {
        input: "Review the current changes".into(),
        label: None,
    };
    app.save_quick_dial(/*slot*/ 3, Some(first)).await;
    let saved = QuickDial {
        input: "Review the current changes".into(),
        label: Some(String::new()),
    };
    assert_eq!(
        (
            app.local_settings.tui.quick_dials.get("3"),
            app.chat_widget.local_settings.tui.quick_dials.get("3"),
        ),
        (Some(&saved), Some(&saved))
    );

    let replacement = QuickDial {
        input: "/model gpt-5.6-terra --reasoning=high --speed=fast".into(),
        label: Some("deep review".into()),
    };
    app.save_quick_dial(/*slot*/ 3, Some(replacement.clone()))
        .await;
    let reloaded = ConfigBuilder::default()
        .codex_home(app.config.codex_home.to_path_buf())
        .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    assert_eq!(reloaded.tui_quick_dials, [("3".into(), replacement)].into());
    assert!(!reloaded.show_tooltips);

    app.save_quick_dial(/*slot*/ 3, /*dial*/ None).await;
    let reloaded = ConfigBuilder::default()
        .codex_home(app.config.codex_home.to_path_buf())
        .loader_overrides(LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    assert!(reloaded.tui_quick_dials.is_empty());
    assert!(app.local_settings.tui.quick_dials.is_empty());
    assert!(app.chat_widget.local_settings.tui.quick_dials.is_empty());
    assert!(!reloaded.show_tooltips);
    Ok(())
}

#[tokio::test]
async fn quick_dials_respect_profile_and_external_ownership() -> Result<()> {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let codex_home = app.config.codex_home.to_path_buf();
    let base_path = app.local_settings.user_config_path.clone();
    let profile_path = AbsolutePathBuf::from_absolute_path(codex_home.join("work.config.toml"))?;
    std::fs::write(
        base_path.as_path(),
        "[tui.presets.1]\ninput = 'base prompt'\nlabel = 'base label'\n",
    )?;
    std::fs::write(
        profile_path.as_path(),
        "[tui.presets.1]\ninput = 'profile prompt'\n",
    )?;
    let mut loader_overrides = LoaderOverrides::without_managed_config_for_tests();
    loader_overrides.user_config_path = Some(profile_path.clone());
    loader_overrides.user_config_profile = Some("work".parse()?);
    let external =
        toml::from_str::<toml::Value>("input = 'externally managed prompt'\nlabel = 'external'\n")?;
    let config = ConfigBuilder::default()
        .codex_home(codex_home)
        .loader_overrides(loader_overrides)
        .cli_overrides(vec![("tui.presets.2".into(), external)])
        .build()
        .await?;
    app.config.config_layer_stack = config.config_layer_stack.clone();
    app.local_settings = crate::local_settings::LocalSettings::from(&config);
    app.chat_widget.local_settings = app.local_settings.clone();

    let replacement = QuickDial {
        input: "replacement prompt".into(),
        label: None,
    };
    app.save_quick_dial(/*slot*/ 1, Some(replacement.clone()))
        .await;
    assert_eq!(
        toml::from_str::<toml::Value>(&std::fs::read_to_string(profile_path.as_path())?)?,
        toml::from_str::<toml::Value>(
            "[tui.presets.1]\ninput = 'replacement prompt'\nlabel = ''\n"
        )?
    );
    assert_eq!(
        std::fs::read_to_string(base_path.as_path())?,
        "[tui.presets.1]\ninput = 'base prompt'\nlabel = 'base label'\n"
    );

    while events.try_recv().is_ok() {}
    let profile_before_rejections = std::fs::read(profile_path.as_path())?;
    let dials_before_rejections = app.local_settings.tui.quick_dials.clone();
    app.save_quick_dial(/*slot*/ 1, /*dial*/ None).await;
    app.save_quick_dial(
        /*slot*/ 2,
        Some(QuickDial {
            input: "local replacement".into(),
            label: None,
        }),
    )
    .await;
    assert_eq!(
        (
            std::fs::read(profile_path.as_path())?,
            &app.local_settings.tui.quick_dials,
            &app.chat_widget.local_settings.tui.quick_dials,
        ),
        (
            profile_before_rejections,
            &dials_before_rejections,
            &dials_before_rejections,
        )
    );
    let errors = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            AppEvent::InsertHistoryCell(cell) => {
                Some(lines_to_single_string(
                    &cell.transcript_lines(/*width*/ 200),
                ))
            }
            _ => None,
        })
        .filter(|message| message.contains("defined outside your user config"))
        .count();
    assert_eq!(errors, 2);
    Ok(())
}

#[tokio::test]
async fn quick_dial_write_failure_leaves_file_and_memory_unchanged() -> Result<()> {
    let (mut app, mut events, _ops) = make_test_app_with_channels().await;
    let config_path = app.local_settings.user_config_path.clone();
    let existing = QuickDial {
        input: "existing prompt".into(),
        label: Some("existing".into()),
    };
    app.local_settings
        .tui
        .quick_dials
        .insert("4".into(), existing);
    app.chat_widget.local_settings = app.local_settings.clone();
    std::fs::write(config_path.as_path(), "[broken")?;
    let file_before = std::fs::read(config_path.as_path())?;
    let dials_before = app.local_settings.tui.quick_dials.clone();
    while events.try_recv().is_ok() {}

    app.save_quick_dial(
        /*slot*/ 4,
        Some(QuickDial {
            input: "replacement prompt".into(),
            label: None,
        }),
    )
    .await;

    assert_eq!(
        (
            std::fs::read(config_path.as_path())?,
            &app.local_settings.tui.quick_dials,
            &app.chat_widget.local_settings.tui.quick_dials,
        ),
        (file_before, &dials_before, &dials_before)
    );
    let messages = std::iter::from_fn(|| events.try_recv().ok())
        .filter_map(|event| match event {
            AppEvent::InsertHistoryCell(cell) => {
                Some(lines_to_single_string(
                    &cell.transcript_lines(/*width*/ 200),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(messages.contains("Failed to update preset 4"));
    assert!(!messages.contains("Saved preset 4"));
    Ok(())
}
