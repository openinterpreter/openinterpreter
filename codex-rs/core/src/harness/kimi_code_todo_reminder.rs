use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::Mutex;

use serde::Deserialize;
use serde_json::Value;
use serde_json::json;

const REMINDER_TURNS: usize = 10;
const REMINDER: &str = "The TodoList tool has not been updated recently. If you are working on tasks that benefit from progress tracking, consider using TodoList to update task status. Also consider clearing or rewriting the todo list if it has become stale and no longer matches the current work. Only use it if relevant. This is a gentle reminder; ignore it if not applicable. Make sure that you NEVER mention this reminder to the user.";
static REMINDER_STATE: LazyLock<Mutex<HashMap<String, ReminderState>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Default)]
struct ReminderState {
    write_marker: String,
    last_reminder_assistant_turn: Option<usize>,
}

#[derive(Deserialize)]
struct TodoItem {
    title: String,
    status: String,
}

pub(super) fn is_todo_list_reminder(message: &Value) -> bool {
    message
        .get("content")
        .and_then(Value::as_str)
        .is_some_and(|content| {
            content.starts_with("<system-reminder>\n") && content.contains(REMINDER)
        })
}

pub(super) fn add_todo_list_reminder(messages: &mut Vec<Value>, conversation_id: &str) {
    let mut assistant_turn: usize = 0;
    let mut latest_write = None;
    for message in messages.iter() {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        assistant_turn += 1;
        let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for tool_call in tool_calls {
            let Some(function) = tool_call.get("function") else {
                continue;
            };
            if function.get("name").and_then(Value::as_str) != Some("TodoList") {
                continue;
            }
            let Some(arguments) = function.get("arguments").and_then(Value::as_str) else {
                continue;
            };
            let Ok(arguments_value) = serde_json::from_str::<Value>(arguments) else {
                continue;
            };
            let Some(todos) = arguments_value.get("todos") else {
                continue;
            };
            let Ok(todos) = serde_json::from_value::<Vec<TodoItem>>(todos.clone()) else {
                continue;
            };
            latest_write = Some((assistant_turn, arguments.to_string(), todos));
        }
    }

    let Some((write_turn, write_arguments, todos)) = latest_write else {
        return;
    };
    let write_marker = format!("{write_turn}:{write_arguments}");
    let turns_since_write = assistant_turn.saturating_sub(write_turn);
    let mut reminder_states = REMINDER_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let reminder_state = reminder_states
        .entry(conversation_id.to_string())
        .or_default();
    if reminder_state.write_marker != write_marker {
        reminder_state.write_marker = write_marker;
        reminder_state.last_reminder_assistant_turn = None;
    }
    if todos.is_empty() || turns_since_write < REMINDER_TURNS {
        return;
    }
    let should_emit = match reminder_state.last_reminder_assistant_turn {
        Some(last_turn) if last_turn == assistant_turn => true,
        Some(last_turn) => assistant_turn.saturating_sub(last_turn) >= REMINDER_TURNS,
        None => true,
    };
    if !should_emit {
        return;
    }
    reminder_state.last_reminder_assistant_turn = Some(assistant_turn);

    let todo_list = todos
        .iter()
        .enumerate()
        .map(|(index, todo)| format!("{}. [{}] {}", index + 1, todo.status, todo.title))
        .collect::<Vec<_>>()
        .join("\n");
    messages.push(json!({
        "role": "user",
        "content": format!(
            "<system-reminder>\n{REMINDER}\n\nCurrent todo list:\n{todo_list}\n</system-reminder>"
        ),
    }));
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::add_todo_list_reminder;

    #[test]
    fn adds_stale_todo_reminder_after_ten_assistant_turns() {
        let mut messages = vec![json!({
            "role": "assistant",
            "tool_calls": [{
                "function": {
                    "name": "TodoList",
                    "arguments": r#"{"todos":[{"title":"Capture state tools","status":"done"},{"title":"Capture remaining tools","status":"in_progress"},{"title":"Finish gauntlet","status":"pending"}]}"#,
                }
            }],
        })];
        messages.extend((0..10).map(|turn| {
            json!({
                "role": "assistant",
                "tool_calls": [{
                    "function": {
                        "name": "Glob",
                        "arguments": format!(r#"{{"pattern":"{turn}"}}"#),
                    }
                }],
            })
        }));

        add_todo_list_reminder(&mut messages, "stale-todo-test");

        assert_eq!(
            messages.last(),
            Some(&json!({
                "role": "user",
                "content": "<system-reminder>\nThe TodoList tool has not been updated recently. If you are working on tasks that benefit from progress tracking, consider using TodoList to update task status. Also consider clearing or rewriting the todo list if it has become stale and no longer matches the current work. Only use it if relevant. This is a gentle reminder; ignore it if not applicable. Make sure that you NEVER mention this reminder to the user.\n\nCurrent todo list:\n1. [done] Capture state tools\n2. [in_progress] Capture remaining tools\n3. [pending] Finish gauntlet\n</system-reminder>",
            }))
        );
    }
}
