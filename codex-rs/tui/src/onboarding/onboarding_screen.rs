use codex_app_server_client::AppServerEvent;
use codex_app_server_client::AppServerRequestHandle;
use codex_app_server_protocol::ServerNotification;
use codex_core::config::Config;
#[cfg(target_os = "windows")]
use codex_core::windows_sandbox::WindowsSandboxLevelExt;
use codex_exec_server::LOCAL_FS;
use codex_git_utils::resolve_root_git_project_for_trust;
#[cfg(target_os = "windows")]
use codex_protocol::config_types::WindowsSandboxLevel;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::terminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Clear;
use ratatui::widgets::Widget;
use ratatui::widgets::WidgetRef;

use codex_protocol::config_types::ForcedLoginMethod;

use crate::LoginStatus;
use crate::app_server_session::AppServerSession;
use crate::onboarding::auth::AuthModeWidget;
use crate::onboarding::auth::SignInOption;
use crate::onboarding::auth::SignInState;
use crate::onboarding::auth::initial_sign_in_state;
use crate::onboarding::provider_setup::default_provider_preset_id;
use crate::onboarding::provider_setup::provider_preset_by_id;
use crate::onboarding::trust_directory::TrustDirectorySelection;
use crate::onboarding::trust_directory::TrustDirectoryWidget;
use crate::onboarding::welcome::WelcomeWidget;
use crate::product_branding::ProductBranding;
use crate::provider_readiness::ProviderReadinessSnapshot;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;
use color_eyre::eyre::Result;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;

#[allow(clippy::large_enum_variant)]
enum Step {
    Welcome(WelcomeWidget),
    Auth(AuthModeWidget),
    TrustDirectory(TrustDirectoryWidget),
}

pub(crate) trait KeyboardHandler {
    fn handle_key_event(&mut self, key_event: KeyEvent);
    fn handle_paste(&mut self, _pasted: String) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepState {
    Hidden,
    InProgress,
    Complete,
}

pub(crate) trait StepStateProvider {
    fn get_step_state(&self) -> StepState;
}

pub(crate) struct OnboardingScreen {
    request_frame: FrameRequester,
    steps: Vec<Step>,
    is_done: bool,
    should_exit: bool,
}

pub(crate) struct OnboardingScreenArgs {
    pub show_trust_screen: bool,
    pub show_provider_step: bool,
    pub login_status: LoginStatus,
    pub app_server_request_handle: Option<AppServerRequestHandle>,
    pub config: Config,
}

pub(crate) struct OnboardingResult {
    pub directory_trust_decision: Option<TrustDirectorySelection>,
    pub should_exit: bool,
}

pub(crate) type DeferredOnboardingAppServerFactory =
    Box<dyn FnMut() -> Pin<Box<dyn Future<Output = Result<AppServerSession>>>>>;

fn used_rows(tmp: &Buffer, width: u16, height: u16) -> u16 {
    if width == 0 || height == 0 {
        return 0;
    }
    let mut last_non_empty: Option<u16> = None;
    for yy in 0..height {
        let mut any = false;
        for xx in 0..width {
            let cell = &tmp[(xx, yy)];
            let has_symbol = !cell.symbol().trim().is_empty();
            let has_style =
                cell.fg != Color::Reset || cell.bg != Color::Reset || !cell.modifier.is_empty();
            if has_symbol || has_style {
                any = true;
                break;
            }
        }
        if any {
            last_non_empty = Some(yy);
        }
    }
    last_non_empty.map(|v| v + 2).unwrap_or(0)
}

impl OnboardingScreen {
    pub(crate) async fn new(tui: &mut Tui, args: OnboardingScreenArgs) -> Self {
        let OnboardingScreenArgs {
            show_trust_screen,
            show_provider_step,
            login_status,
            app_server_request_handle,
            config,
        } = args;
        let cwd = config.cwd.to_path_buf();
        let codex_home = config.codex_home.to_path_buf();
        let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
        let forced_login_method = config.forced_login_method;
        let force_provider_onboarding = crate::should_force_provider_onboarding();
        let branding = ProductBranding::current();
        let mut steps: Vec<Step> = Vec::new();
        steps.push(Step::Welcome(WelcomeWidget::new(
            !matches!(login_status, LoginStatus::NotAuthenticated),
            tui.frame_requester(),
            config.animations,
        )));
        #[cfg(target_os = "windows")]
        let show_windows_create_sandbox_hint =
            WindowsSandboxLevel::from_config(&config) == WindowsSandboxLevel::Disabled;
        #[cfg(not(target_os = "windows"))]
        let show_windows_create_sandbox_hint = false;
        let highlighted = TrustDirectorySelection::Trust;
        if show_trust_screen {
            let trust_target = resolve_root_git_project_for_trust(LOCAL_FS.as_ref(), &config.cwd)
                .await
                .map(Into::into)
                .unwrap_or_else(|| cwd.clone());
            steps.push(Step::TrustDirectory(TrustDirectoryWidget {
                cwd: cwd.clone(),
                trust_target,
                codex_home: codex_home.clone(),
                show_windows_create_sandbox_hint,
                should_quit: false,
                selection: None,
                highlighted,
                error: None,
            }))
        }
        if show_provider_step {
            let highlighted_mode = match forced_login_method {
                Some(ForcedLoginMethod::Api) => SignInOption::ApiKey,
                _ => SignInOption::ChatGpt,
            };
            let highlighted_provider_id = if force_provider_onboarding {
                default_provider_preset_id()
            } else {
                provider_preset_by_id(config.model_provider_id.as_str())
                    .map(|preset| preset.provider_id)
                    .unwrap_or_else(default_provider_preset_id)
            };
            steps.push(Step::Auth(AuthModeWidget {
                request_frame: tui.frame_requester(),
                interpreter_home: config.codex_home.to_path_buf(),
                highlighted_mode,
                highlighted_provider_id,
                provider_filter_query: String::new(),
                configured_model_providers: config.model_providers.clone(),
                current_model_provider_id: if force_provider_onboarding {
                    String::new()
                } else {
                    config.model_provider_id.clone()
                },
                current_model: if force_provider_onboarding {
                    None
                } else {
                    config.model.clone()
                },
                imported_model_provider_id: if force_provider_onboarding {
                    Some(config.model_provider_id.clone())
                } else {
                    None
                },
                imported_model: if force_provider_onboarding {
                    config.model.clone()
                } else {
                    None
                },
                suppress_current_provider: force_provider_onboarding,
                provider_readiness_snapshot: ProviderReadinessSnapshot::from_system(&config),
                error: Arc::new(RwLock::new(None)),
                sign_in_state: Arc::new(RwLock::new(initial_sign_in_state(
                    branding,
                    forced_login_method,
                ))),
                branding,
                login_status,
                app_server_request_handle,
                pending_app_server_request: Arc::new(RwLock::new(None)),
                forced_chatgpt_workspace_id,
                forced_login_method,
                animations_enabled: config.animations,
                animations_suppressed: std::cell::Cell::new(false),
                provider_login_abort: Arc::new(Mutex::new(None)),
            }));
        }
        // TODO: add git warning.
        Self {
            request_frame: tui.frame_requester(),
            steps,
            is_done: false,
            should_exit: false,
        }
    }

    fn current_steps_mut(&mut self) -> Vec<&mut Step> {
        let mut out: Vec<&mut Step> = Vec::new();
        for step in self.steps.iter_mut() {
            match step.get_step_state() {
                StepState::Hidden => continue,
                StepState::Complete => out.push(step),
                StepState::InProgress => {
                    out.push(step);
                    break;
                }
            }
        }
        out
    }

    fn current_steps(&self) -> Vec<&Step> {
        let mut out: Vec<&Step> = Vec::new();
        for step in self.steps.iter() {
            match step.get_step_state() {
                StepState::Hidden => continue,
                StepState::Complete => out.push(step),
                StepState::InProgress => {
                    out.push(step);
                    break;
                }
            }
        }
        out
    }

    fn is_auth_in_progress(&self) -> bool {
        self.steps.iter().any(|step| {
            matches!(step, Step::Auth(_)) && matches!(step.get_step_state(), StepState::InProgress)
        })
    }

    pub(crate) fn is_done(&self) -> bool {
        self.is_done
            || !self
                .steps
                .iter()
                .any(|step| matches!(step.get_step_state(), StepState::InProgress))
    }

    pub fn directory_trust_decision(&self) -> Option<TrustDirectorySelection> {
        self.steps
            .iter()
            .find_map(|step| {
                if let Step::TrustDirectory(TrustDirectoryWidget { selection, .. }) = step {
                    Some(*selection)
                } else {
                    None
                }
            })
            .flatten()
    }

    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn cancel_auth_if_active(&self) {
        for step in &self.steps {
            if let Step::Auth(widget) = step {
                widget.cancel_active_attempt();
            }
        }
    }

    fn auth_widget_mut(&mut self) -> Option<&mut AuthModeWidget> {
        self.steps.iter_mut().find_map(|step| match step {
            Step::Auth(widget) => Some(widget),
            Step::Welcome(_) | Step::TrustDirectory(_) => None,
        })
    }

    fn handle_app_server_notification(&mut self, notification: ServerNotification) {
        match notification {
            ServerNotification::AccountLoginCompleted(notification) => {
                if let Some(widget) = self.auth_widget_mut() {
                    widget.on_account_login_completed(notification);
                }
            }
            ServerNotification::AccountUpdated(notification) => {
                if let Some(widget) = self.auth_widget_mut() {
                    widget.on_account_updated(notification);
                }
            }
            _ => {}
        }
    }

    fn is_text_entry_active(&self) -> bool {
        self.steps.iter().any(|step| {
            if let Step::Auth(widget) = step {
                return widget.sign_in_state.read().is_ok_and(|g| {
                    matches!(
                        &*g,
                        SignInState::ProviderPicker
                            | SignInState::ProviderSetup(_)
                            | SignInState::ProviderModelSelection(_)
                            | SignInState::ManualModelEntry(_)
                            | SignInState::ApiKeyEntry(_)
                    )
                });
            }
            false
        })
    }

    fn desired_height(&self, width: u16, max_height: u16) -> u16 {
        if width == 0 || max_height == 0 {
            return 1;
        }
        let scratch_area = Rect::new(0, 0, width, max_height);
        let mut scratch = Buffer::empty(scratch_area);
        self.render_ref(scratch_area, &mut scratch);
        used_rows(&scratch, width, max_height).clamp(1, max_height)
    }

    fn needs_app_server_connection(&self) -> bool {
        self.steps.iter().any(|step| {
            if let Step::Auth(widget) = step {
                return widget.needs_app_server_connection();
            }
            false
        })
    }

    fn attach_app_server_request_handle(&mut self, request_handle: AppServerRequestHandle) {
        if let Some(widget) = self.auth_widget_mut() {
            widget.attach_app_server_request_handle(request_handle);
        }
    }

    fn on_app_server_connection_failed(&mut self, error_message: String) {
        if let Some(widget) = self.auth_widget_mut() {
            widget.on_app_server_connection_failed(error_message);
        }
    }
}

impl KeyboardHandler for OnboardingScreen {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        let is_text_entry_active = self.is_text_entry_active();
        let should_quit = match key_event {
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: crossterm::event::KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: crossterm::event::KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            } => true,
            KeyEvent {
                code: KeyCode::Char('q'),
                kind: KeyEventKind::Press,
                ..
            } => !is_text_entry_active,
            _ => false,
        };
        if should_quit {
            if self.is_auth_in_progress() {
                self.cancel_auth_if_active();
                // If the user cancels the auth menu, exit the app rather than
                // leave the user at a prompt in an unauthed state.
                self.should_exit = true;
            }
            self.is_done = true;
        } else {
            if let Some(Step::Welcome(widget)) = self
                .steps
                .iter_mut()
                .find(|step| matches!(step, Step::Welcome(_)))
            {
                widget.handle_key_event(key_event);
            }
            if let Some(active_step) = self.current_steps_mut().into_iter().last() {
                active_step.handle_key_event(key_event);
            }
            if self.steps.iter().any(|step| {
                if let Step::TrustDirectory(widget) = step {
                    widget.should_quit()
                } else {
                    false
                }
            }) {
                self.should_exit = true;
                self.is_done = true;
            }
        }
        self.request_frame.schedule_frame();
    }

    fn handle_paste(&mut self, pasted: String) {
        if pasted.is_empty() {
            return;
        }

        if let Some(active_step) = self.current_steps_mut().into_iter().last() {
            active_step.handle_paste(pasted);
        }
        self.request_frame.schedule_frame();
    }
}

impl WidgetRef for &OnboardingScreen {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        // Render steps top-to-bottom, measuring each step's height dynamically.
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);
        let width = area.width;

        let mut i = 0usize;
        let current_steps = self.current_steps();

        while i < current_steps.len() && y < bottom {
            let step = &current_steps[i];
            let max_h = bottom.saturating_sub(y);
            if max_h == 0 || width == 0 {
                break;
            }
            let scratch_area = Rect::new(0, 0, width, max_h);
            let mut scratch = Buffer::empty(scratch_area);
            if let Step::Welcome(widget) = step {
                widget.update_layout_area(scratch_area);
            }
            step.render_ref(scratch_area, &mut scratch);
            let h = used_rows(&scratch, width, max_h).min(max_h);
            if h > 0 {
                let target = Rect {
                    x: area.x,
                    y,
                    width,
                    height: h,
                };
                step.render_ref(target, buf);
                y = y.saturating_add(h);
            }
            i += 1;
        }
    }
}

impl KeyboardHandler for Step {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self {
            Step::Welcome(widget) => widget.handle_key_event(key_event),
            Step::Auth(widget) => widget.handle_key_event(key_event),
            Step::TrustDirectory(widget) => widget.handle_key_event(key_event),
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        match self {
            Step::Welcome(_) => {}
            Step::Auth(widget) => widget.handle_paste(pasted),
            Step::TrustDirectory(widget) => widget.handle_paste(pasted),
        }
    }
}

impl StepStateProvider for Step {
    fn get_step_state(&self) -> StepState {
        match self {
            Step::Welcome(w) => w.get_step_state(),
            Step::Auth(w) => w.get_step_state(),
            Step::TrustDirectory(w) => w.get_step_state(),
        }
    }
}

impl WidgetRef for Step {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        match self {
            Step::Welcome(widget) => {
                widget.render_ref(area, buf);
            }
            Step::Auth(widget) => {
                widget.render_ref(area, buf);
            }
            Step::TrustDirectory(widget) => {
                widget.render_ref(area, buf);
            }
        }
    }
}

pub(crate) async fn run_onboarding_app(
    args: OnboardingScreenArgs,
    mut app_server: Option<AppServerSession>,
    mut deferred_app_server: Option<DeferredOnboardingAppServerFactory>,
    tui: &mut Tui,
) -> Result<OnboardingResult> {
    use tokio_stream::StreamExt;

    let mut onboarding_screen = OnboardingScreen::new(tui, args).await;
    let screen_height = terminal::size()
        .map(|(_, height)| height)
        .unwrap_or(u16::MAX);
    let screen_width = terminal::size().map(|(width, _)| width).unwrap_or(u16::MAX);
    let mut onboarding_height = onboarding_screen.desired_height(screen_width, screen_height);

    tui.draw(onboarding_height, |frame| {
        frame.render_widget_ref(&onboarding_screen, frame.area());
    })?;

    let tui_events = tui.event_stream();
    tokio::pin!(tui_events);
    let mut app_server_connect_future: Option<
        Pin<Box<dyn Future<Output = Result<AppServerSession>>>>,
    > = None;

    while !onboarding_screen.is_done() {
        if app_server.is_none()
            && app_server_connect_future.is_none()
            && onboarding_screen.needs_app_server_connection()
            && let Some(factory) = deferred_app_server.as_mut()
        {
            app_server_connect_future = Some(factory());
        }

        tokio::select! {
            result = async {
                match app_server_connect_future.as_mut() {
                    Some(future) => Some(future.as_mut().await),
                    None => None,
                }
            }, if app_server.is_none() && app_server_connect_future.is_some() => {
                let Some(result) = result else {
                    continue;
                };
                app_server_connect_future = None;
                match result {
                    Ok(connected_app_server) => {
                        let request_handle = connected_app_server.request_handle();
                        onboarding_screen.attach_app_server_request_handle(request_handle);
                        app_server = Some(connected_app_server);
                    }
                    Err(err) => {
                        onboarding_screen.on_app_server_connection_failed(err.to_string());
                    }
                }
            }
            event = tui_events.next() => {
                if let Some(event) = event {
                    match event {
                        TuiEvent::Key(key_event) => {
                            onboarding_screen.handle_key_event(key_event);
                        }
                        TuiEvent::Paste(text) => {
                            onboarding_screen.handle_paste(text);
                        }
                        TuiEvent::Draw => {
                            let (screen_width, screen_height) =
                                terminal::size().unwrap_or((u16::MAX, u16::MAX));
                            onboarding_height =
                                onboarding_screen.desired_height(screen_width, screen_height);
                            let _ = tui.draw(onboarding_height, |frame| {
                                frame.render_widget_ref(&onboarding_screen, frame.area());
                            });
                        }
                    }
                }
            }
            event = async {
                match app_server.as_mut() {
                    Some(app_server) => app_server.next_event().await,
                    None => None,
                }
            }, if app_server.is_some() => {
                if let Some(event) = event {
                    match event {
                        AppServerEvent::ServerNotification(notification) => {
                            onboarding_screen.handle_app_server_notification(notification);
                        }
                        AppServerEvent::Disconnected { message } => {
                            return Err(color_eyre::eyre::eyre!(message));
                        }
                        AppServerEvent::Lagged { .. }
                        | AppServerEvent::ServerRequest(_) => {}
                    }
                }
            }
        }
    }
    if let Some(app_server) = app_server {
        app_server.shutdown().await.ok();
    }
    Ok(OnboardingResult {
        directory_trust_decision: onboarding_screen.directory_trust_decision(),
        should_exit: onboarding_screen.should_exit(),
    })
}
