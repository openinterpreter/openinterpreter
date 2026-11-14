use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::time::Duration;

use chrono::DateTime;
use chrono::Local;
use chrono::SecondsFormat;
use chrono::TimeZone;
use chrono::Timelike;
use chrono::Utc;
use codex_protocol::ThreadId;
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::context::ContextualUserFragment;
use crate::context::KimiCronFire;
use crate::current_time::TimeProvider;
use crate::kimi_cron_expression::CronExpression;
use crate::kimi_cron_expression::next_cron_run;
use crate::kimi_cron_expression::parse_cron_expression;
use crate::session::session::Session;

#[path = "kimi_cron_storage.rs"]
mod storage;
use storage::generate_unique_id;
use storage::is_valid_id;
use storage::persist_task;

const MAX_CRON_JOBS_PER_SESSION: usize = 50;
const MAX_PROMPT_BYTES: usize = 8 * 1024;
const ONE_SHOT_MAX_FUTURE_MS: i64 = 350 * 24 * 60 * 60 * 1000;
const STALE_THRESHOLD_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const MAX_COALESCED_FIRES: usize = 10_000;

#[derive(Clone)]
pub(crate) struct KimiCronService {
    inner: Arc<KimiCronInner>,
}

struct KimiCronInner {
    state: Mutex<KimiCronState>,
    storage_dir: PathBuf,
    wake_scheduler: Notify,
    cancellation_token: CancellationToken,
    scheduler_task: Mutex<Option<JoinHandle<()>>>,
    time_provider: Arc<dyn TimeProvider>,
    thread_id: ThreadId,
}

#[derive(Default)]
struct KimiCronState {
    tasks: Vec<CronTask>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CronTask {
    id: String,
    cron: String,
    prompt: String,
    recurring: bool,
    created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_fired_at: Option<i64>,
}

#[derive(Clone, Debug)]
struct CronDelivery {
    task: CronTask,
    cursor_after_fire: i64,
    coalesced_count: usize,
    stale: bool,
}

impl KimiCronService {
    pub(crate) fn new(
        storage_dir: PathBuf,
        time_provider: Arc<dyn TimeProvider>,
        thread_id: ThreadId,
    ) -> Self {
        Self {
            inner: Arc::new(KimiCronInner {
                state: Mutex::new(KimiCronState::default()),
                storage_dir,
                wake_scheduler: Notify::new(),
                cancellation_token: CancellationToken::new(),
                scheduler_task: Mutex::new(None),
                time_provider,
                thread_id,
            }),
        }
    }

    pub(crate) async fn start(&self, session: Weak<Session>) {
        self.load_from_disk().await;
        let service = self.clone();
        let task = tokio::spawn(async move {
            service.run_scheduler(session).await;
        });
        *self.inner.scheduler_task.lock().await = Some(task);
        self.inner.wake_scheduler.notify_one();
    }

    pub(crate) async fn shutdown(&self) {
        self.inner.cancellation_token.cancel();
        self.inner.wake_scheduler.notify_waiters();
        let task = self.inner.scheduler_task.lock().await.take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }

    pub(crate) async fn create(
        &self,
        cron: &str,
        prompt: String,
        recurring: bool,
    ) -> Result<String, String> {
        if std::env::var("KIMI_DISABLE_CRON").as_deref() == Ok("1") {
            return Err("Cron scheduling is disabled (KIMI_DISABLE_CRON=1).".to_string());
        }
        let expression = parse_cron_expression(cron)
            .map_err(|error| format!("Invalid cron expression: {error}"))?;
        if prompt.is_empty() {
            return Err("Prompt must not be empty.".to_string());
        }
        let prompt_bytes = prompt.len();
        if prompt_bytes > MAX_PROMPT_BYTES {
            return Err(format!(
                "Prompt exceeds {MAX_PROMPT_BYTES} bytes (got {prompt_bytes})."
            ));
        }
        let now_ms = self.current_time_ms().await?;
        let ideal = next_cron_run(&expression, now_ms).ok_or_else(|| {
            format!(
                "Cron expression {:?} has no fire within 5 years; refusing to schedule.",
                expression.raw
            )
        })?;
        if !recurring && ideal.saturating_sub(now_ms) > ONE_SHOT_MAX_FUTURE_MS {
            return Err(format!(
                "One-shot cron {:?} would not fire until {} (more than a year out). If you meant \"today\" or a near date, the pinned day/month has already passed this year — pick a future date or use wildcards.",
                expression.raw,
                format_local_timestamp(ideal)
            ));
        }

        let task = {
            let mut state = self.inner.state.lock().await;
            if state.tasks.len() >= MAX_CRON_JOBS_PER_SESSION {
                return Err(format!(
                    "Cron job cap reached (max {MAX_CRON_JOBS_PER_SESSION} per session)."
                ));
            }
            let id = generate_unique_id(&state.tasks)?;
            let task = CronTask {
                id,
                cron: expression.raw.clone(),
                prompt,
                recurring,
                created_at: now_ms,
                last_fired_at: None,
            };
            state.tasks.push(task.clone());
            task
        };
        self.persist_task(&task).await;
        self.inner.wake_scheduler.notify_one();

        let next_fire_at = jittered_fire_time(&task, &expression, ideal);
        Ok([
            format!("id: {}", task.id),
            format!("cron: {}", task.cron),
            format!("humanSchedule: {}", expression.human_schedule()),
            format!("recurring: {}", task.recurring),
            format!("nextFireAt: {}", format_local_timestamp(next_fire_at)),
        ]
        .join("\n"))
    }

    pub(crate) async fn delete(&self, id: &str) -> Result<String, String> {
        if !is_valid_id(id) {
            return Err(format!(
                "Invalid cron job id {id:?} — must be 8 lowercase hex characters."
            ));
        }
        let removed = {
            let mut state = self.inner.state.lock().await;
            state
                .tasks
                .iter()
                .position(|task| task.id == id)
                .map(|index| state.tasks.remove(index))
        };
        let Some(task) = removed else {
            return Err(format!("No cron job with id {id}."));
        };
        self.remove_persisted_task(&task.id).await;
        Ok(format!("Deleted cron job {id}."))
    }

    pub(crate) async fn list(&self) -> String {
        let now_ms = self
            .current_time_ms()
            .await
            .unwrap_or_else(|_| Utc::now().timestamp_millis());
        let state = self.inner.state.lock().await;
        if state.tasks.is_empty() {
            return "cron_jobs: 0\nNo cron jobs scheduled.".to_string();
        }
        let records = state
            .tasks
            .iter()
            .map(|task| render_task(task, now_ms))
            .collect::<Vec<_>>();
        format!("cron_jobs: {}\n{}", records.len(), records.join("\n---\n"))
    }

    async fn run_scheduler(&self, session: Weak<Session>) {
        loop {
            if !self.has_tasks().await {
                tokio::select! {
                    _ = self.inner.cancellation_token.cancelled() => break,
                    _ = self.inner.wake_scheduler.notified() => {}
                }
                continue;
            }
            tokio::select! {
                _ = self.inner.cancellation_token.cancelled() => break,
                _ = self.inner.time_provider.sleep(self.inner.thread_id, Duration::from_secs(1)) => {}
                _ = self.inner.wake_scheduler.notified() => {}
            }
            if self.inner.cancellation_token.is_cancelled()
                || std::env::var("KIMI_DISABLE_CRON").as_deref() == Ok("1")
            {
                continue;
            }
            let Some(session) = session.upgrade() else {
                break;
            };
            let now_ms = match self.current_time_ms().await {
                Ok(now_ms) => now_ms,
                Err(error) => {
                    warn!("failed to read Kimi cron clock: {error}");
                    continue;
                }
            };
            let Some(delivery) = self.next_due_delivery(now_ms).await else {
                continue;
            };
            let input = ContextualUserFragment::into(KimiCronFire::new(
                &delivery.task.id,
                &delivery.task.cron,
                delivery.task.recurring,
                delivery.coalesced_count,
                delivery.stale,
                &delivery.task.prompt,
            ));
            if session.inject_if_running(vec![input]).await.is_ok() {
                self.complete_delivery(delivery).await;
            }
        }
    }

    async fn has_tasks(&self) -> bool {
        !self.inner.state.lock().await.tasks.is_empty()
    }

    async fn current_time_ms(&self) -> Result<i64, String> {
        self.inner
            .time_provider
            .current_time(self.inner.thread_id)
            .await
            .map(|time| time.timestamp_millis())
            .map_err(|error| {
                format!("Unable to read the current time for cron scheduling: {error}")
            })
    }

    async fn next_due_delivery(&self, now_ms: i64) -> Option<CronDelivery> {
        let state = self.inner.state.lock().await;
        state
            .tasks
            .iter()
            .find_map(|task| delivery_for_task(task, now_ms))
    }

    async fn complete_delivery(&self, delivery: CronDelivery) {
        let persistence = {
            let mut state = self.inner.state.lock().await;
            let index = state
                .tasks
                .iter()
                .position(|task| task.id == delivery.task.id);
            match index {
                Some(index) if !delivery.task.recurring || delivery.stale => {
                    Some((state.tasks.remove(index), true))
                }
                Some(index) => {
                    state.tasks[index].last_fired_at = Some(delivery.cursor_after_fire);
                    Some((state.tasks[index].clone(), false))
                }
                None => None,
            }
        };
        match persistence {
            Some((task, true)) => self.remove_persisted_task(&task.id).await,
            Some((task, false)) => self.persist_task(&task).await,
            None => {}
        }
    }

    async fn load_from_disk(&self) {
        let mut entries = match tokio::fs::read_dir(&self.inner.storage_dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                warn!("failed to read Kimi cron storage: {error}");
                return;
            }
        };
        let mut tasks = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            match tokio::fs::read(&path).await {
                Ok(bytes) => match serde_json::from_slice::<CronTask>(&bytes) {
                    Ok(task) if is_valid_id(&task.id) => tasks.push(task),
                    Ok(_) => warn!("ignored malformed Kimi cron task at {}", path.display()),
                    Err(error) => warn!("ignored unreadable Kimi cron task: {error}"),
                },
                Err(error) => warn!("failed to read Kimi cron task: {error}"),
            }
        }
        tasks.sort_by_key(|task| (task.created_at, task.id.clone()));
        self.inner.state.lock().await.tasks = tasks;
    }

    async fn persist_task(&self, task: &CronTask) {
        if let Err(error) = persist_task(&self.inner.storage_dir, task).await {
            warn!("failed to persist Kimi cron task {}: {error}", task.id);
        }
    }

    async fn remove_persisted_task(&self, id: &str) {
        let path = self.inner.storage_dir.join(format!("{id}.json"));
        if let Err(error) = tokio::fs::remove_file(&path).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            warn!(
                "failed to remove Kimi cron task {}: {error}",
                path.display()
            );
        }
    }
}

fn delivery_for_task(task: &CronTask, now_ms: i64) -> Option<CronDelivery> {
    let expression = parse_cron_expression(&task.cron).ok()?;
    let base = task
        .last_fired_at
        .unwrap_or(task.created_at)
        .max(task.created_at);
    let first_ideal = next_cron_run(&expression, base)?;
    if now_ms < jittered_fire_time(task, &expression, first_ideal) {
        return None;
    }
    let stale = task.recurring && now_ms.saturating_sub(task.created_at) >= STALE_THRESHOLD_MS;
    let (coalesced_count, cursor_after_fire) = if task.recurring {
        count_coalesced(task, &expression, first_ideal, now_ms)
    } else {
        (1, first_ideal)
    };
    Some(CronDelivery {
        task: task.clone(),
        cursor_after_fire,
        coalesced_count,
        stale,
    })
}

fn count_coalesced(
    task: &CronTask,
    expression: &CronExpression,
    first_ideal: i64,
    now_ms: i64,
) -> (usize, i64) {
    let mut count = 1;
    let mut cursor = first_ideal;
    while count < MAX_COALESCED_FIRES {
        let Some(next) = next_cron_run(expression, cursor) else {
            break;
        };
        if next > now_ms || jittered_fire_time(task, expression, next) > now_ms {
            break;
        }
        count += 1;
        cursor = next;
    }
    (count, cursor)
}

fn jittered_fire_time(task: &CronTask, expression: &CronExpression, ideal_ms: i64) -> i64 {
    if std::env::var("KIMI_CRON_NO_JITTER").as_deref() == Ok("1") {
        return ideal_ms;
    }
    let fraction =
        u32::from_str_radix(&task.id, 16).unwrap_or_default() as f64 / (u32::MAX as f64 + 1.0);
    if task.recurring {
        let period = next_cron_run(expression, ideal_ms)
            .map(|next| next.saturating_sub(ideal_ms))
            .unwrap_or(24 * 60 * 60 * 1000);
        let cap = ((period as f64) * 0.1).min(15.0 * 60_000.0);
        return ideal_ms.saturating_add((cap * fraction) as i64);
    }
    let Some(timestamp) = Local.timestamp_millis_opt(ideal_ms).single() else {
        return ideal_ms;
    };
    if !matches!(timestamp.minute(), 0 | 30) {
        return ideal_ms;
    }
    let shifted = ideal_ms.saturating_sub((90_000.0 * fraction) as i64);
    if shifted < task.created_at {
        ideal_ms
    } else {
        shifted
    }
}

fn render_task(task: &CronTask, now_ms: i64) -> String {
    let expression = parse_cron_expression(&task.cron).ok();
    let next_fire_at = expression.as_ref().and_then(|expression| {
        let base = task
            .last_fired_at
            .unwrap_or(task.created_at)
            .max(task.created_at);
        next_cron_run(expression, base).map(|ideal| jittered_fire_time(task, expression, ideal))
    });
    let human_schedule = expression
        .as_ref()
        .map_or_else(|| task.cron.clone(), CronExpression::human_schedule);
    let prompt = preview_prompt(&task.prompt);
    let age_days = now_ms.saturating_sub(task.created_at) as f64 / (24.0 * 60.0 * 60_000.0);
    let stale = task.recurring && now_ms.saturating_sub(task.created_at) >= STALE_THRESHOLD_MS;
    [
        format!("id: {}", task.id),
        format!("cron: {}", task.cron),
        format!("humanSchedule: {human_schedule}"),
        format!(
            "prompt: {}",
            serde_json::to_string(&prompt).unwrap_or_else(|_| "\"\"".to_string())
        ),
        format!(
            "nextFireAt: {}",
            next_fire_at.map_or_else(|| "null".to_string(), format_local_timestamp)
        ),
        format!("recurring: {}", task.recurring),
        format!("ageDays: {age_days:.2}"),
        format!("stale: {stale}"),
    ]
    .join("\n")
}

fn preview_prompt(prompt: &str) -> String {
    if prompt.len() <= 200 {
        return prompt.to_string();
    }
    let mut end = 200;
    while !prompt.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(truncated)", &prompt[..end])
}

fn format_local_timestamp(timestamp_ms: i64) -> String {
    Local
        .timestamp_millis_opt(timestamp_ms)
        .single()
        .map(|timestamp: DateTime<Local>| timestamp.to_rfc3339_opts(SecondsFormat::Millis, false))
        .unwrap_or_else(|| "null".to_string())
}

#[cfg(test)]
#[path = "kimi_cron_tests.rs"]
mod tests;
