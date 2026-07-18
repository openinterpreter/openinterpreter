use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::Mutex;

static SYSTEM_PROMPTS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn get_or_insert(
    conversation_id: &str,
    build_prompt: impl FnOnce() -> String,
) -> String {
    let Ok(home) = crate::config::find_codex_home() else {
        let memory_key = format!("<memory>:{conversation_id}");
        let mut prompts = SYSTEM_PROMPTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return prompts
            .entry(memory_key)
            .or_insert_with(build_prompt)
            .clone();
    };
    let cache_root = home
        .join("kimi-code")
        .join("session-prompts")
        .join(env!("CARGO_PKG_VERSION"));
    get_or_insert_at(&cache_root, conversation_id, build_prompt)
}

fn get_or_insert_at(
    cache_root: &Path,
    conversation_id: &str,
    build_prompt: impl FnOnce() -> String,
) -> String {
    let memory_key = format!("{}:{conversation_id}", cache_root.display());
    if let Some(cached) = SYSTEM_PROMPTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&memory_key)
        .cloned()
    {
        return cached;
    }

    let safe_conversation_id = conversation_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cache_path = cache_root.join(format!("{safe_conversation_id}.md"));
    if let Ok(cached) = std::fs::read_to_string(&cache_path) {
        SYSTEM_PROMPTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(memory_key, cached.clone());
        return cached;
    }

    let mut rendered = build_prompt();
    if std::fs::create_dir_all(cache_root).is_ok() {
        let temporary_path = cache_path.with_extension(format!("md.tmp-{}", std::process::id()));
        if let Ok(mut temporary_file) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            if temporary_file.write_all(rendered.as_bytes()).is_ok()
                && temporary_file.flush().is_ok()
            {
                drop(temporary_file);
                if std::fs::rename(&temporary_path, &cache_path).is_err() {
                    if let Ok(cached) = std::fs::read_to_string(&cache_path) {
                        rendered = cached;
                    }
                    let _ = std::fs::remove_file(temporary_path);
                }
            } else {
                drop(temporary_file);
                let _ = std::fs::remove_file(temporary_path);
            }
        } else if let Ok(cached) = std::fs::read_to_string(&cache_path) {
            rendered = cached;
        }
    }

    SYSTEM_PROMPTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(memory_key, rendered.clone());
    rendered
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::SYSTEM_PROMPTS;
    use super::get_or_insert_at;

    #[test]
    fn session_prompt_survives_an_empty_memory_cache() {
        let cache = tempfile::tempdir().expect("temp cache");
        let conversation_id = "persistent-session-prompt";
        let memory_key = format!("{}:{conversation_id}", cache.path().display());
        let initial = get_or_insert_at(cache.path(), conversation_id, || "initial".to_string());

        SYSTEM_PROMPTS
            .lock()
            .expect("system prompt cache lock")
            .remove(&memory_key);
        let after_memory_reset =
            get_or_insert_at(cache.path(), conversation_id, || "changed".to_string());

        assert_eq!(initial, "initial");
        assert_eq!(after_memory_reset, initial);
    }
}
