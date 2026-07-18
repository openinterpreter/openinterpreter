use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub(super) struct KimiTodo {
    pub(super) title: String,
    pub(super) status: KimiTodoStatus,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum KimiTodoStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum GoalBudgetUnit {
    Turns,
    Tokens,
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
}

pub(super) fn render_todo_list(todos: &[KimiTodo]) -> String {
    if todos.is_empty() {
        return "Todo list is empty.".to_string();
    }
    let mut lines = vec!["Current todo list:".to_string()];
    lines.extend(todos.iter().map(|todo| {
        let status = match todo.status {
            KimiTodoStatus::Pending => "pending",
            KimiTodoStatus::InProgress => "in_progress",
            KimiTodoStatus::Done => "done",
        };
        format!("  [{status}] {}", todo.title)
    }));
    lines.join("\n")
}

pub(super) fn format_elapsed(milliseconds: u64) -> String {
    let seconds = (milliseconds.saturating_add(500)) / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m{:02}s", seconds % 60);
    }
    format!("{}h{:02}m", minutes / 60, minutes % 60)
}

pub(super) fn format_budget(value: f64, unit: GoalBudgetUnit) -> String {
    let value = if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    };
    let unit = match unit {
        GoalBudgetUnit::Turns => "turns",
        GoalBudgetUnit::Tokens => "tokens",
        GoalBudgetUnit::Milliseconds => "milliseconds",
        GoalBudgetUnit::Seconds => "seconds",
        GoalBudgetUnit::Minutes => "minutes",
        GoalBudgetUnit::Hours => "hours",
    };
    let unit = if value == "1" {
        unit.strip_suffix('s').unwrap_or(unit)
    } else {
        unit
    };
    format!("{value} {unit}")
}

pub(super) fn format_tokens(tokens: i64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 1_000_000 {
        return format!("{:.1}k", tokens as f64 / 1_000.0);
    }
    format!("{:.1}M", tokens as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::KimiTodo;
    use super::KimiTodoStatus;
    use super::format_elapsed;
    use super::format_tokens;
    use super::render_todo_list;

    #[test]
    fn renders_kimi_todo_state() {
        assert_eq!(
            render_todo_list(&[
                KimiTodo {
                    title: "First".to_string(),
                    status: KimiTodoStatus::Done,
                },
                KimiTodo {
                    title: "Second".to_string(),
                    status: KimiTodoStatus::InProgress,
                },
            ]),
            "Current todo list:\n  [done] First\n  [in_progress] Second"
        );
    }

    #[test]
    fn formats_kimi_goal_usage() {
        assert_eq!(format_elapsed(/*milliseconds*/ 20_600), "21s");
        assert_eq!(format_elapsed(/*milliseconds*/ 65_000), "1m05s");
        assert_eq!(format_tokens(/*tokens*/ 281), "281");
        assert_eq!(format_tokens(/*tokens*/ 4_300), "4.3k");
    }
}
