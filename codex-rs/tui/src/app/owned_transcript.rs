//! Compose the owned transcript above the composer and route their selection gestures.
//! Reserve a cleared row between transcript content and the composer. Slash suggestions overlay
//! already-painted rows so opening or closing them leaves transcript geometry unchanged.
//! Plain Enter returns an empty composer to latest after transcript interactions and prompt editing.

use super::side_pane;
use super::sidebar;
use super::sidebar::PaneFocus;
use super::turn_tips::TipSurface;
use super::*;
use crate::history_cell::HistoryRenderMode;
use crate::keymap::KeymapContext;
use crate::keymap::bindings_for_action;
use crate::keymap::keymap_action_ids;
use crate::motion::MotionMode;
use crate::pager_overlay::TranscriptHistoryState;
use crate::sidebar_tab_strip::TabState;
use crate::transcript_view::ViewAction;
use ratatui::widgets::Widget;

/// Geometry from the last completed draw, used to validate pointer targets
/// against what was actually visible rather than an asynchronous sidebar update.
#[derive(Default)]
pub(super) struct ConversationFrame {
    inset: Option<Rect>,
    pub(super) sidebar: Option<sidebar::SidebarLayout>,
    pub(super) chat_strip: Option<Rect>,
    pub(super) sidebar_input: Option<sidebar::SidebarInputState>,
    pub(super) close_buttons: [Rect; 2],
}

impl ConversationFrame {
    #[cfg(test)]
    pub(super) fn set_caller_inset(&mut self, inset: Rect) {
        self.inset = Some(inset);
    }

    fn begin_layout(&mut self) -> Option<Rect> {
        // Only a caller's inset survives a draw. A sidebar or tab strip must be
        // recalculated from its current model and screen dimensions.
        if self.sidebar.is_some() || self.chat_strip.is_some() {
            *self = Self::default();
        }
        self.inset
    }

    fn record_layout(
        &mut self,
        caller_inset: Option<Rect>,
        main: Rect,
        screen: Rect,
        sidebar: Option<sidebar::SidebarLayout>,
        chat_strip: Option<Rect>,
    ) {
        self.inset = caller_inset.or_else(|| (!main.is_empty() && main != screen).then_some(main));
        self.sidebar = sidebar;
        self.chat_strip = chat_strip;
    }

    pub(super) fn has_caller_inset(&self) -> bool {
        self.inset.is_some() && self.sidebar.is_none() && self.chat_strip.is_none()
    }

    pub(super) fn side_pane(&self) -> Option<&side_pane::SidePaneLayout> {
        match &self.sidebar {
            Some(sidebar::SidebarLayout::Chat(panes)) => Some(panes),
            _ => None,
        }
    }

    pub(super) fn chooser_covers_transcript(&self) -> bool {
        matches!(self.sidebar, Some(sidebar::SidebarLayout::Chooser { main, .. }) if main.is_empty())
    }

    pub(super) fn chooser_area(&self) -> Option<Rect> {
        match self.sidebar {
            Some(sidebar::SidebarLayout::Chooser { sidebar, .. }) => Some(sidebar),
            _ => None,
        }
    }

    pub(super) fn sidebar_tab_strip(&self) -> Option<Rect> {
        self.chat_strip.or_else(|| {
            self.chooser_area()
                .map(|area| side_pane::tab_and_body(area)[0])
        })
    }

    pub(super) fn divider_area(&self) -> Option<Rect> {
        self.sidebar.as_ref()?.divider_area()
    }
}

#[cfg(test)]
mod conversation_frame_tests {
    use super::*;

    #[test]
    fn sidebar_inset_does_not_become_a_caller_inset_on_next_draw() {
        let screen = Rect::new(0, 0, 160, 40);
        let main = Rect::new(0, 0, 100, 40);
        let side = Rect::new(101, 0, 59, 40);
        let mut frame = ConversationFrame::default();
        frame.record_layout(
            None,
            main,
            screen,
            Some(sidebar::SidebarLayout::Chooser {
                main,
                sidebar: side,
                focus: PaneFocus::Sidebar,
            }),
            None,
        );
        assert_eq!(frame.chooser_area(), Some(side));
        assert_eq!(
            frame.sidebar_tab_strip(),
            Some(side_pane::tab_and_body(side)[0])
        );
        assert_eq!(frame.divider_area(), Some(Rect::new(100, 0, 1, 40)));
        assert!(!frame.has_caller_inset());
        assert_eq!(frame.begin_layout(), None);
        assert_eq!(frame.chooser_area(), None);

        let caller = Rect::new(10, 2, 110, 25);
        frame.record_layout(Some(caller), caller, screen, None, None);
        assert!(frame.has_caller_inset());
        assert_eq!(frame.begin_layout(), Some(caller));
    }
}

impl App {
    pub(super) fn conversation_area(&self, viewport: Rect) -> Rect {
        self.conversation_frame
            .inset
            .map_or(viewport, |area| area.intersection(viewport))
    }

    pub(super) fn refresh_owned_transcript(&mut self, tui: &mut tui::Tui) -> Result<()> {
        let size = tui.prepare_draw_size()?;
        let rendered_area = self.render_owned_transcript(tui, size)?;
        self.draw_pet_images(tui, size, rendered_area)
    }

    /// Copy draft selections before key shortcuts, or after mouse layout has been refreshed.
    pub(super) fn handle_composer_copy_event(
        &mut self,
        tui: &mut tui::Tui,
        event: &TuiEvent,
        copy: impl FnOnce(&mut tui::Tui, &str) -> Result<crate::clipboard_copy::CopyStatus, String>,
    ) -> bool {
        if tui.is_owned_screen()
            && self.overlay.is_none()
            && !self.transcript_view.has_active_interaction()
            && let Some((char_count, result)) = self
                .chat_widget
                .copy_composer_selection(event, |text| copy(tui, text))
        {
            self.cancel_pending_key_chord();
            self.transcript_view.show_copy_feedback(&result, char_count);
            tui.frame_requester().schedule_frame();
            return true;
        }
        false
    }

    fn sync_owned_transcript(&mut self, width: u16) -> bool {
        let chat_widget = &self.chat_widget;
        let transcript_width = chat_widget.history_wrap_width(width);
        let view = &mut self.transcript_view;
        view.set_keymap_bindings(&self.keymap);
        view.set_presentation(view.is_detailed(), chat_widget.history_render_mode());
        let active_key = chat_widget.active_cell_transcript_key();
        let detailed = view.is_detailed();
        let active_ids = chat_widget.active_activity_ids();
        let expanded = view.sync_live_activity(&self.transcript_cells, active_ids)
            && chat_widget.history_render_mode() == HistoryRenderMode::Rich;
        if detailed {
            view.sync_live_tail(transcript_width, active_key, |width| {
                chat_widget.active_cell_transcript_hyperlink_lines(width)
            })
        } else {
            view.sync_live_activity_tail(transcript_width, active_key, expanded, |width| {
                chat_widget.active_cell_owned_transcript_lines(width, expanded)
            })
        }
    }

    pub(super) fn render_owned_transcript(
        &mut self,
        tui: &mut tui::Tui,
        screen_size: Size,
    ) -> Result<Rect> {
        let screen: Rect = screen_size.into();
        let caller_inset = self.conversation_frame.begin_layout();
        // Keep the last painted input regions in the frame, not in the sidebar's
        // asynchronously mutable model. The same settled split paints both panes.
        let layout = sidebar::paint_layout(self, tui, screen);
        let displayed = self.current_displayed_thread_id();
        let chat = match displayed.and_then(|id| self.side_pair_key(id)) {
            Some(child) if Some(child) == displayed => TabState::Active,
            Some(_) => TabState::Inactive,
            None => TabState::Absent,
        };
        let (main, strip) = match &layout {
            Some(sidebar::SidebarLayout::Chat(panes)) => (panes.focused, Some(panes.tab_strip)),
            Some(sidebar::SidebarLayout::Chooser { main, .. }) => (*main, None),
            None if chat == TabState::Active
                && caller_inset.is_none()
                && self.side_retention_enabled(tui) =>
            {
                let [strip, body] = side_pane::tab_and_body(screen);
                (body, Some(strip))
            }
            None => (
                caller_inset.map_or(screen, |inset| inset.intersection(screen)),
                None,
            ),
        };
        self.conversation_frame
            .record_layout(caller_inset, main, screen, layout, strip);
        let parent = self.sidebar_parent();
        self.conversation_frame.sidebar_input = self
            .sidebars
            .get(&parent)
            .map(sidebar::SidebarPane::input_state);
        let divider = self.conversation_frame.divider_area();
        if self.divider_drag.is_some_and(|drag| {
            divider.is_none()
                || !drag.matches(screen, self)
                || !self.conversation_focus_available(tui)
        }) {
            self.divider_drag = None;
        }
        let highlighted_divider =
            self.pane_resize.is_some() || tui.link_hover.shape == tui::PointerShape::Resize;
        let hovered_target = self.hovered_sidebar_target(tui.link_hover.position());
        let mut hud = self.workspace_hud(tui, screen);
        if let Some(hud) = &mut hud {
            hud.viewport = caller_inset;
        }
        let frames = tui.frame_requester();
        self.chat_widget.sync_warnings(&self.transcript_cells);
        let motion = MotionMode::from_animations_enabled(
            self.local_settings.tui.animations && self.local_settings.tui.effects.shimmer,
        );
        let focused = tui.is_terminal_focused();
        self.empty_state_presentation(
            MotionMode::from_animations_enabled(
                self.local_settings.tui.animations && self.local_settings.tui.effects.welcome,
            ),
            focused,
        );
        let latest_navigation = if self.enter_returns_to_latest() {
            "enter/esc latest"
        } else {
            "esc latest"
        };
        self.sync_owned_transcript(main.width);
        let transcript_width = self
            .chat_widget
            .history_wrap_width(main.width)
            .min(main.width);
        let composer_hint = self.composer_hint(transcript_width);
        // A tip is chosen before the draw but only counted once its dedicated row
        // was actually painted. In particular, a clipped completion tip must not
        // consume the cadence or become transcript source text.
        let turn_tip = self.turn_tip(transcript_width, Instant::now(), &frames);
        let mut prompt_footer = self.prompt_navigation_footer(main.width.saturating_sub(/*rhs*/ 2));
        let pane = self.sidebars.get(&parent);
        let side_companions = &mut self.side_companions;
        let conversation_frame = &mut self.conversation_frame;
        let keymap = &self.keymap;
        let settings = &self.local_settings.tui;
        let divider_drag = self.divider_drag;
        let pane_resize = self.pane_resize;
        let chat_widget = &self.chat_widget;
        let view = &mut self.transcript_view;
        let active_key = chat_widget.active_cell_transcript_key();
        view.sync_history_tail(&self.transcript_cells);
        view.prepare_width(transcript_width);
        if view.advance_search(&self.transcript_cells) {
            tui.frame_requester().schedule_frame();
        }
        let footer_area = crate::bottom_pane::inset_footer_hint_area(Rect::new(
            main.x, main.y, main.width, /*height*/ 1,
        ));
        let footer = view.footer_with_navigation(footer_area.width, motion, latest_navigation);
        let bottom = chat_widget.bottom_pane_renderable(
            prompt_footer.as_ref().or(footer.as_ref()),
            if view.has_active_interaction() {
                crate::bottom_pane::CommandPopupPlacement::Hidden
            } else {
                crate::bottom_pane::CommandPopupPlacement::Overlay
            },
        );
        let dashboard_visible = chat_widget
            .selected_index_for_present_view(AGENTS_OVERVIEW_VIEW_ID)
            .is_some();
        let bottom_height = if dashboard_visible {
            main.height
        } else {
            bottom.desired_height(main.width).min(main.height)
        };
        drop(bottom);
        let available = main.height.saturating_sub(bottom_height);
        let bottom_area = Rect::new(
            main.x,
            main.bottom().saturating_sub(bottom_height),
            main.width,
            bottom_height,
        );
        let mut rendered_cursor = None;
        let mut footer_height_changed = false;
        let mut feedback_tick = None;
        let mut tip_was_painted = false;
        tui.draw_with_shortcut_hud(screen_size.height, hud, |frame| {
            chat_widget.rendered_selection.borrow_mut().areas.clear();
            ratatui::widgets::Clear.render(
                Rect::new(main.x, main.y, main.width, available),
                frame.buffer,
            );
            let transcript_area = Rect::new(
                main.x,
                main.y,
                transcript_width,
                available.saturating_sub(/*rhs*/ 1),
            );
            let completion_tip = turn_tip
                .as_ref()
                .and_then(|(surface, tip)| (*surface == TipSurface::Completion).then_some(tip));
            let tip_area = view.render_with_turn_tip_space(
                transcript_area,
                frame.buffer,
                &self.transcript_cells,
                completion_tip,
            );
            if let (Some(tip), Some(area)) = (completion_tip, tip_area) {
                tip.render(area, frame.buffer);
            }
            let follow_area =
                (available > 0 && chat_widget.no_modal_or_popup_active()).then(|| {
                    Rect::new(
                        main.x,
                        main.y + available - 1,
                        transcript_width,
                        /*height*/ 1,
                    )
                });
            feedback_tick =
                view.render_composer_gap(follow_area, composer_hint.as_ref(), frame.buffer);
            // Working tips sit adjacent to the composer. They never replace
            // history and yield to a visible follow/copy affordance.
            if let (Some((TipSurface::Working, tip)), Some(area)) = (turn_tip.as_ref(), follow_area)
                && composer_hint.is_none()
            {
                tip.render(area, frame.buffer);
            }
            if let Some((_, tip)) = turn_tip.as_ref()
                && let Some(area) = tip.rendered.get()
            {
                let mut selection = chat_widget.rendered_selection.borrow_mut();
                selection.register(area, /*invalidate*/ false);
                selection.render(frame.buffer);
                tip_was_painted = true;
            }
            // Rendering resolves whether new activity is still hidden. Paint that result in
            // this frame so a revision change cannot flash a stale activity hint.
            let mut footer =
                view.footer_with_navigation(footer_area.width, motion, latest_navigation);
            if let Some(footer) = prompt_footer
                .as_mut()
                .or(footer.as_mut())
                .filter(|footer| footer.is_interactive)
                && let Some(items) = self
                    .key_chord_matcher
                    .pending_hint_items(&self.keymap.chords, footer_area.width)
                && let Some(progress) = footer.text.lines.last_mut()
            {
                *progress = crate::bottom_pane::footer_hint_items_line(&items);
            }
            let bottom = chat_widget.bottom_pane_renderable(
                prompt_footer.as_ref().or(footer.as_ref()),
                if view.has_active_interaction() {
                    crate::bottom_pane::CommandPopupPlacement::Hidden
                } else {
                    crate::bottom_pane::CommandPopupPlacement::Overlay
                },
            );
            footer_height_changed = !dashboard_visible
                && bottom.desired_height(main.width).min(main.height) != bottom_height;
            bottom.render(bottom_area, frame.buffer);
            chat_widget.note_rendered_width(main.width);
            rendered_cursor = bottom.cursor_pos(bottom_area);
            if let Some(position) = rendered_cursor {
                frame.set_cursor_style(bottom.cursor_style(bottom_area));
                frame.set_cursor_position(position);
            }
            match &conversation_frame.sidebar {
                Some(sidebar::SidebarLayout::Chat(panes)) => {
                    if let Some(companion) = side_companions.get_mut(&panes.passive_thread) {
                        companion.view.image_protocol = view.image_protocol;
                        panes.paint_companion(
                            companion,
                            frame,
                            settings.collapse_turn_details,
                            &frames,
                        );
                    }
                }
                Some(sidebar::SidebarLayout::Chooser { sidebar, focus, .. }) => {
                    let focused = *focus == PaneFocus::Sidebar;
                    let resize_hint = pane_resize.map(|_| side_pane::RESIZE_HINT);
                    if let Some(creation) = pane.and_then(|pane| pane.creation.as_ref()) {
                        use sidebar::CreationState;
                        ratatui::widgets::Clear.render(*sidebar, frame.buffer);
                        let mut body = sidebar.inner(ratatui::layout::Margin::new(1, 1));
                        if let Some(hint) = resize_hint {
                            let footer = crate::sidebar_chooser::body_row(*sidebar, sidebar.y);
                            crate::sidebar_chooser::resize_hint(hint, footer.width)
                                .render(footer, frame.buffer);
                            body.height = body.height.saturating_sub(1);
                        }
                        let text = match &creation.state {
                            CreationState::Waiting { started, .. } => {
                                let mode = MotionMode::from_animations_enabled(settings.animations);
                                let glyph = crate::motion::loading_glyph(*started, mode, &frames);
                                format!("{glyph} Starting side chat…")
                            }
                            CreationState::Failed { message, .. } => {
                                if resize_hint.is_some() {
                                    message.clone()
                                } else {
                                    crate::sidebar_chooser::creation_failure_text(message, keymap)
                                }
                            }
                        };
                        ratatui::widgets::Paragraph::new(text)
                            .wrap(ratatui::widgets::Wrap { trim: false })
                            .render(body, frame.buffer);
                    } else if let Some(pane) = pane {
                        let footer = if let Some(hint) = resize_hint {
                            let row = crate::sidebar_chooser::body_row(*sidebar, sidebar.y);
                            crate::sidebar_chooser::resize_hint(hint, row.width)
                        } else {
                            crate::sidebar_chooser::footer(
                                keymap,
                                *sidebar,
                                pane.choice,
                                pane.chat_tabs(),
                                chat_widget.side_chat_creation_state(),
                            )
                            .0
                            .line
                        };
                        crate::sidebar_chooser::paint(
                            focused,
                            *sidebar,
                            frame.buffer,
                            footer,
                            pane.chat_tabs(),
                            pane.choice,
                            &crate::sidebar_chooser::SidebarStyle {
                                new_chat: chat_widget.side_chat_creation_state(),
                                use_theme_colors: settings.status_line_use_colors,
                                motion: MotionMode::from_animations_enabled(settings.animations),
                                frames: &frames,
                            },
                        );
                        crate::sidebar_chooser::tab_strip(
                            *sidebar,
                            if focused {
                                TabState::Active
                            } else {
                                TabState::Inactive
                            },
                            TabState::Absent,
                            pane.chat_tabs(),
                            hovered_target,
                            frame.buffer,
                        );
                    }
                    frame
                        .image_placements
                        .retain(|image| !sidebar.intersects(image.area));
                    if focused {
                        frame.cursor_position = None;
                    }
                }
                None => {}
            }
            if let Some(strip) = strip
                && let Some(pane) = pane
            {
                crate::sidebar_chooser::tab_strip(
                    strip,
                    TabState::Absent,
                    chat,
                    pane.chat_tabs(),
                    hovered_target,
                    frame.buffer,
                );
            }
            if let Some(divider) = divider {
                side_pane::paint_divider(frame, divider.x, divider_drag, highlighted_divider);
            }
            conversation_frame.close_buttons = Default::default();
            if let Some(close) = pane.and_then(|pane| pane.close.as_ref()) {
                let viewport = frame.area();
                let column = conversation_frame
                    .chooser_area()
                    .or(strip)
                    .unwrap_or(viewport);
                let area = Rect {
                    x: column.x,
                    width: column.width,
                    ..viewport
                };
                conversation_frame.close_buttons = close.paint(area, frame.buffer);
                frame
                    .image_placements
                    .retain(|image| !area.intersects(image.area));
                frame.cursor_position = None;
            }
        })?;
        if tip_was_painted && let Some((surface, _)) = turn_tip.as_ref() {
            self.turn_tips.acknowledge(*surface);
        }
        if footer_height_changed {
            tui.frame_requester().schedule_frame();
        }
        if let Some(delay) = feedback_tick {
            tui.frame_requester().schedule_frame_in(delay);
        }
        let animating =
            view.is_following() && active_key.is_some_and(|key| key.animation_tick.is_some());
        let loading = view.is_loading_history() && motion == MotionMode::Animated;
        if animating || loading {
            tui.frame_requester()
                .schedule_frame_in(Duration::from_millis(/*millis*/ 50));
        }
        Ok(bottom_area)
    }

    /// Consume transcript gestures only when a modal or another overlay does not own input.
    pub(super) fn handle_owned_transcript_event(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
        event: &TuiEvent,
    ) -> Result<bool> {
        if !tui.is_owned_screen()
            || matches!(event, TuiEvent::FocusLost | TuiEvent::Resume)
            || self.overlay.is_some()
            || !self.chat_widget.no_modal_or_popup_active()
        {
            self.transcript_view.end_drag();
        }
        if !tui.is_owned_screen()
            || matches!(event, TuiEvent::FocusLost | TuiEvent::Resume)
            || self.overlay.is_some()
        {
            self.chat_widget.end_composer_drag();
        }
        if !tui.is_owned_screen() || self.overlay.is_some() {
            return Ok(false);
        }
        if matches!(event, TuiEvent::FocusLost) {
            // Show the static, faded decoration immediately when the terminal loses focus.
            tui.frame_requester().schedule_frame();
        }
        if self.transcript_view.history == TranscriptHistoryState::Idle {
            self.transcript_view.history = if self.scrollback_has_older_history {
                TranscriptHistoryState::Partial
            } else {
                TranscriptHistoryState::Complete
            };
        }
        if matches!(event, TuiEvent::Draw) {
            if self.transcript_view.tick_selection(&self.transcript_cells) {
                tui.frame_requester()
                    .schedule_frame_in(crate::tui::TARGET_FRAME_INTERVAL);
            }
            return Ok(false);
        }
        if let TuiEvent::Mouse(mouse) = event {
            let composer_ready = self.chat_widget.prepare_composer_mouse(*mouse);
            if mouse.kind != crossterm::event::MouseEventKind::Moved {
                let size = tui.prepare_draw_size()?;
                self.render_owned_transcript(tui, size)?;
            }
            if composer_ready
                && self.handle_composer_copy_event(tui, event, tui::Tui::copy_transcript_selection)
            {
                return Ok(true);
            }
            if composer_ready && self.chat_widget.handle_composer_mouse(*mouse) {
                self.transcript_view.end_selection(&self.transcript_cells);
                self.transcript_view.cancel_search();
                self.transcript_view.clear_activity_focus();
                tui.frame_requester().schedule_frame();
                return Ok(true);
            }
        }
        if !self.chat_widget.no_modal_or_popup_active() {
            self.chat_widget.end_composer_drag();
            return Ok(false);
        }
        if matches!(event, TuiEvent::Key(key) if key.kind != KeyEventKind::Release) {
            let size = tui.prepare_draw_size()?;
            self.render_owned_transcript(tui, size)?;
        }
        if matches!(event, TuiEvent::Key(_))
            || matches!(event, TuiEvent::Mouse(mouse) if matches!(mouse.kind, crossterm::event::MouseEventKind::Down(_)))
        {
            self.chat_widget.end_composer_drag();
        }
        // Visible shortcut help owns Escape before returning a paused viewport to latest.
        // A transcript search or selection still owns Escape while it replaces the help footer.
        if let TuiEvent::Key(key) = event
            && !self.transcript_view.has_active_interaction()
            && self.chat_widget.shortcut_overlay_visible()
            && crate::key_hint::plain(KeyCode::Esc).is_press(*key)
        {
            return Ok(false);
        }
        if let TuiEvent::Key(key) = event
            && !self.transcript_view.has_active_interaction()
            && self.chat_widget.should_handle_vim_insert_escape(*key)
        {
            return Ok(false);
        }
        // Prompt preview owns its navigation before ordinary Escape-to-latest scrolling.
        // Selection and search retain their existing priority and keep the prompt untouched.
        if self.backtrack.overlay_preview_active
            && !self.transcript_view.has_active_interaction()
            && self.handle_owned_backtrack_event(tui, event)?
        {
            self.request_owned_history(tui, app_server);
            return Ok(true);
        }
        // Capture the origin before the first Escape returns a paused viewport to latest.
        if !self.backtrack.overlay_preview_active
            && !self.transcript_view.has_active_interaction()
            && !self.reconnect.offline
            && let TuiEvent::Key(key) = event
            && key.code == KeyCode::Esc
            && key.modifiers.is_empty()
            && key.kind == KeyEventKind::Press
            && self.should_handle_backtrack_esc(*key)
        {
            if !self.handle_owned_backtrack_event(tui, event)? {
                self.handle_backtrack_esc_key(tui);
                if !self.backtrack.overlay_preview_active {
                    self.transcript_view.jump_to_latest();
                }
            }
            tui.frame_requester().schedule_frame();
            return Ok(true);
        }
        if let TuiEvent::Key(key) = event
            && !self.transcript_view.has_active_interaction()
            && !self.backtrack.overlay_preview_active
            && self.transcript_view.is_detailed()
            && self.keymap.pager.close_transcript.is_pressed(*key)
        {
            self.close_transcript_overlay(tui);
            return Ok(true);
        }
        if let TuiEvent::Key(key) = event
            && ((key.modifiers.is_empty()
                && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown))
                || crate::transcript_view::JumpTarget::from_key(*key).is_some())
            && !self.transcript_view.has_active_interaction()
            && keymap_action_ids().any(|action| {
                action.context != KeymapContext::Pager
                    && !matches!(action.action, "find_transcript" | "focus_activity")
                    && self.active_keymap_contexts().contains_action(action)
                    && bindings_for_action(
                        &self.keymap,
                        action.context.config_name(),
                        action.action,
                    )
                    .is_some_and(|bindings| bindings.is_pressed(*key))
            })
        {
            return Ok(false);
        }
        let action = match event {
            TuiEvent::Key(key)
                if !self.transcript_view.is_search_active()
                    && self.keymap.app.find_transcript.is_pressed(*key) =>
            {
                self.transcript_view.begin_search();
                Some(ViewAction::Changed)
            }
            TuiEvent::Key(key) => self
                .transcript_view
                .handle_key(*key, &self.transcript_cells),
            TuiEvent::Mouse(mouse) => self
                .transcript_view
                .handle_mouse(*mouse, &self.transcript_cells),
            TuiEvent::Paste(text) => {
                self.transcript_view.end_selection(&self.transcript_cells);
                if self.transcript_view.paste_search(text) {
                    Some(ViewAction::Changed)
                } else {
                    self.transcript_view.clear_activity_focus();
                    None
                }
            }
            TuiEvent::Draw
            | TuiEvent::Resume
            | TuiEvent::Resize(_)
            | TuiEvent::FocusGained
            | TuiEvent::FocusLost => None,
        };
        let Some(action) = action else {
            if matches!(
                event,
                TuiEvent::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    ..
                })
            ) && self.enter_returns_to_latest()
                && self.transcript_view.can_return_to_latest()
            {
                self.transcript_view.jump_to_latest();
                tui.frame_requester().schedule_frame();
                return Ok(true);
            }
            if self.reconnect.offline {
                if let TuiEvent::Key(key) = event
                    && self.transcript_view.is_detailed()
                    && self.keymap.pager.close_transcript.is_pressed(*key)
                {
                    self.close_transcript_overlay(tui);
                    return Ok(true);
                }
                return Ok(false);
            }
            return self.handle_owned_backtrack_event(tui, event);
        };
        let resume_following = matches!(action, ViewAction::CopyAndFollow(_));
        match action {
            ViewAction::Changed => {}
            ViewAction::Copy(text) | ViewAction::CopyAndFollow(text) => {
                let result = self.transcript_view.copy_selected_text_with(
                    &self.transcript_cells,
                    &text,
                    |text| tui.copy_transcript_selection(text),
                );
                self.transcript_view
                    .show_copy_feedback(&result, text.chars().count());
                if resume_following
                    && matches!(result, Ok(crate::clipboard_copy::CopyStatus::Confirmed))
                {
                    if self.backtrack.overlay_preview_active {
                        self.close_transcript_overlay(tui);
                    }
                    self.transcript_view.jump_to_latest();
                }
            }
            ViewAction::CopyOnSelect(_) | ViewAction::PrimarySelection(_) => {
                if let Some((characters, result)) = self.transcript_view.copy_action(
                    tui,
                    &self.transcript_cells,
                    &action,
                    crate::clipboard_copy::CopyFormat::PlainText,
                ) {
                    self.transcript_view.show_copy_feedback(&result, characters);
                }
            }
            // Select mode has already ended its pinned revision while handling the key.
            ViewAction::ExitSelectMode => {}
            ViewAction::OpenLink(url) => self.open_url_in_browser(url),
        }
        self.request_owned_history(tui, app_server);
        tui.frame_requester().schedule_frame();
        Ok(true)
    }

    /// Keep the navigation hint and input guard aligned, including pending paste and attachments.
    fn enter_returns_to_latest(&self) -> bool {
        self.chat_widget.composer_is_empty()
            && self.chat_widget.no_modal_or_popup_active()
            && !self.backtrack.primed
            && !self.backtrack.overlay_preview_active
    }

    pub(super) fn request_owned_history(
        &mut self,
        tui: &mut tui::Tui,
        app_server: &mut AppServerSession,
    ) {
        if !self.scrollback_has_older_history {
            return;
        }
        let browsing_needs_history = self.browsing_needs_history();
        let view = &mut self.transcript_view;
        if !browsing_needs_history
            && !view.needs_history(&self.transcript_cells)
            && view.history != TranscriptHistoryState::LoadingBeginning
        {
            return;
        }
        if let Some(thread_id) = self.chat_widget.thread_id()
            && app_server.has_older_history(thread_id)
            && self.request_older_history_page(app_server, thread_id)
        {
            if self.transcript_view.history != TranscriptHistoryState::LoadingBeginning {
                self.transcript_view.history = TranscriptHistoryState::LoadingOlder;
            }
            tui.frame_requester().schedule_frame();
        }
    }
}

#[cfg(test)]
#[path = "owned_transcript_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "empty_state_animation_tests.rs"]
mod empty_state_animation_tests;

#[cfg(test)]
#[path = "owned_transcript_input_tests.rs"]
mod input_tests;

#[cfg(test)]
#[path = "warning_notice_tests.rs"]
mod warning_notice_tests;

#[cfg(test)]
#[path = "owned_transcript_follow_tests.rs"]
mod follow_tests;
