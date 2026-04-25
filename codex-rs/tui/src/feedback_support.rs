#[cfg(any(feature = "feedback", feature = "embedded-app-server"))]
pub(crate) use codex_feedback::CodexFeedback;
#[cfg(any(feature = "feedback", feature = "embedded-app-server"))]
pub(crate) use codex_feedback::FEEDBACK_DIAGNOSTICS_ATTACHMENT_FILENAME;
#[cfg(any(feature = "feedback", feature = "embedded-app-server"))]
pub(crate) use codex_feedback::FeedbackDiagnostics;

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
use std::collections::HashMap;

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
use codex_protocol::ThreadId;
#[cfg(all(
    not(any(feature = "feedback", feature = "embedded-app-server")),
    feature = "logging"
))]
use tracing::Subscriber;
#[cfg(all(
    not(any(feature = "feedback", feature = "embedded-app-server")),
    feature = "logging"
))]
use tracing_subscriber::Layer;

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
pub(crate) const FEEDBACK_DIAGNOSTICS_ATTACHMENT_FILENAME: &str =
    "codex-connectivity-diagnostics.txt";
#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
const PROXY_ENV_VARS: &[&str] = &[
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
];

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FeedbackDiagnostic {
    pub headline: String,
    pub details: Vec<String>,
}

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FeedbackDiagnostics {
    diagnostics: Vec<FeedbackDiagnostic>,
}

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
impl FeedbackDiagnostics {
    pub(crate) fn new(diagnostics: Vec<FeedbackDiagnostic>) -> Self {
        Self { diagnostics }
    }

    pub(crate) fn collect_from_env() -> Self {
        let env = std::env::vars().collect::<HashMap<_, _>>();
        let proxy_details = PROXY_ENV_VARS
            .iter()
            .filter_map(|key| env.get(*key).map(|value| format!("{key} = {value}")))
            .collect::<Vec<_>>();

        if proxy_details.is_empty() {
            Self::default()
        } else {
            Self {
                diagnostics: vec![FeedbackDiagnostic {
                    headline: "Proxy environment variables are set and may affect connectivity."
                        .to_string(),
                    details: proxy_details,
                }],
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub(crate) fn diagnostics(&self) -> &[FeedbackDiagnostic] {
        &self.diagnostics
    }

    pub(crate) fn attachment_text(&self) -> Option<String> {
        if self.diagnostics.is_empty() {
            return None;
        }

        let mut lines = vec!["Connectivity diagnostics".to_string(), String::new()];
        for diagnostic in &self.diagnostics {
            lines.push(format!("- {}", diagnostic.headline));
            lines.extend(
                diagnostic
                    .details
                    .iter()
                    .map(|detail| format!("  - {detail}")),
            );
        }
        Some(lines.join("\n"))
    }
}

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
#[derive(Clone, Default)]
pub(crate) struct CodexFeedback;

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
impl CodexFeedback {
    pub(crate) fn new() -> Self {
        Self
    }

    #[cfg(feature = "logging")]
    pub(crate) fn logger_layer(&self) -> NoopLayer {
        NoopLayer
    }

    #[cfg(feature = "logging")]
    pub(crate) fn metadata_layer(&self) -> NoopLayer {
        NoopLayer
    }

    pub(crate) fn snapshot(&self, session_id: Option<ThreadId>) -> FeedbackSnapshot {
        FeedbackSnapshot {
            feedback_diagnostics: FeedbackDiagnostics::collect_from_env(),
            thread_id: session_id
                .map(|id| id.to_string())
                .unwrap_or("no-active-thread".to_string()),
        }
    }
}

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
pub(crate) struct FeedbackSnapshot {
    feedback_diagnostics: FeedbackDiagnostics,
    pub thread_id: String,
}

#[cfg(not(any(feature = "feedback", feature = "embedded-app-server")))]
impl FeedbackSnapshot {
    pub(crate) fn feedback_diagnostics(&self) -> &FeedbackDiagnostics {
        &self.feedback_diagnostics
    }
}

#[cfg(all(
    not(any(feature = "feedback", feature = "embedded-app-server")),
    feature = "logging"
))]
#[derive(Clone, Copy, Default)]
pub(crate) struct NoopLayer;

#[cfg(all(
    not(any(feature = "feedback", feature = "embedded-app-server")),
    feature = "logging"
))]
impl<S> Layer<S> for NoopLayer where S: Subscriber {}
