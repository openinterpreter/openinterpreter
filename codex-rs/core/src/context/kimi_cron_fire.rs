use super::ContextualUserFragment;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::input_text;

/// A scheduled Kimi Code prompt re-injected into its owning thread.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct KimiCronFire {
    job_id: String,
    cron: String,
    recurring: bool,
    coalesced_count: usize,
    stale: bool,
    prompt: String,
}

impl KimiCronFire {
    pub(crate) fn new(
        job_id: impl Into<String>,
        cron: impl Into<String>,
        recurring: bool,
        coalesced_count: usize,
        stale: bool,
        prompt: impl Into<String>,
    ) -> Self {
        Self {
            job_id: job_id.into(),
            cron: cron.into(),
            recurring,
            coalesced_count,
            stale,
            prompt: prompt.into(),
        }
    }
}

impl ContextualUserFragment for KimiCronFire {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("kimi.cron_fire".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<cron-fire", "</cron-fire>")
    }

    fn body(&self) -> ContentItem {
        let job_id = escape_attribute(&self.job_id);
        let cron = escape_attribute(&self.cron);
        input_text(format!(
            " jobId=\"{job_id}\" cron=\"{cron}\" recurring=\"{}\" coalescedCount=\"{}\" stale=\"{}\">\n<prompt>\n{}\n</prompt>\n",
            self.recurring, self.coalesced_count, self.stale, self.prompt
        ))
    }
}

fn escape_attribute(value: &str) -> String {
    value.replace('&', "&amp;").replace('"', "&quot;")
}
