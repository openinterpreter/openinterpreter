//! Helpers for surfacing the session's `<skills_instructions>` developer
//! block in emulated harness requests.
//!
//! The workspace harness instruction-role rule says runtime skills are
//! assembled above the harness layer as a `<skills_instructions>` developer
//! block in `prompt.input` and must never be dropped per-harness; each harness
//! maps the block to the closest shape it supports. These helpers locate the
//! block in the prompt input and parse its `### Available skills` entries so
//! harnesses can re-render them in their native skills format.

use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_CLOSE_TAG;
use codex_protocol::protocol::SKILLS_INSTRUCTIONS_OPEN_TAG;

/// A skill entry parsed from the `### Available skills` list of the session's
/// `<skills_instructions>` developer block. Entries render natively as
/// `- name: description (file: path)`; `path` may be an absolute path or an
/// `rN/...` alias path when the skills list was rendered under budget
/// pressure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SessionSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) path: String,
}

/// Returns the full `<skills_instructions>...</skills_instructions>` text of
/// the session's skills developer block, if the prompt input carries one.
pub(crate) fn find_skills_instructions_text(items: &[ResponseItem]) -> Option<&str> {
    items.iter().find_map(|item| {
        let ResponseItem::Message { role, content, .. } = item else {
            return None;
        };
        if role != "developer" {
            return None;
        }
        content.iter().find_map(|content_item| {
            let ContentItem::InputText { text } = content_item else {
                return None;
            };
            skills_instructions_body(text).map(|_| text.as_str())
        })
    })
}

/// Strips the `<skills_instructions>` tags and surrounding newlines, returning
/// the native body (a `## Skills` markdown section).
pub(crate) fn skills_instructions_body(text: &str) -> Option<&str> {
    let body = text
        .trim()
        .strip_prefix(SKILLS_INSTRUCTIONS_OPEN_TAG)?
        .strip_suffix(SKILLS_INSTRUCTIONS_CLOSE_TAG)?
        .trim_matches('\n');
    (!body.is_empty()).then_some(body)
}

/// Parses the session's skills from the `<skills_instructions>` developer
/// block in the prompt input. Returns an empty list when the session has no
/// skills block.
pub(crate) fn parse_session_skills(items: &[ResponseItem]) -> Vec<SessionSkill> {
    find_skills_instructions_text(items)
        .map(parse_skills_instructions)
        .unwrap_or_default()
}

fn parse_skills_instructions(text: &str) -> Vec<SessionSkill> {
    let Some(body) = skills_instructions_body(text) else {
        return Vec::new();
    };
    let Some((_, rest)) = body.split_once("### Available skills\n") else {
        return Vec::new();
    };
    let section = rest
        .split_once("\n### How to use skills")
        .map_or(rest, |(section, _)| section);

    // Skill descriptions can contain newlines, so an entry may span multiple
    // lines. Every entry ends with its `(file: ...)` suffix, so accumulate
    // lines until the entry parses.
    let mut skills = Vec::new();
    let mut entry = String::new();
    for line in section.lines() {
        if !entry.is_empty() {
            entry.push('\n');
        }
        entry.push_str(line);
        if let Some(skill) = parse_skill_entry(&entry) {
            skills.push(skill);
            entry.clear();
        }
    }
    skills
}

/// Parses one `- name: description (file: path)` entry. The description may be
/// empty (`- name: (file: path)`) or span multiple lines.
fn parse_skill_entry(entry: &str) -> Option<SessionSkill> {
    let rest = entry.trim().strip_prefix("- ")?;
    let (head, path) = rest.strip_suffix(')')?.rsplit_once(" (file: ")?;
    // Skill names can contain colons (`plugin:skill`) but no spaces, so the
    // name/description separator is the first `": "`.
    let (name, description) = match head.split_once(": ") {
        Some((name, description)) => (name, description),
        None => (head.trim_end().strip_suffix(':')?, ""),
    };
    let name = name.trim();
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    Some(SessionSkill {
        name: name.to_string(),
        description: description.trim().to_string(),
        path: path.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn skills_block(lines: &str) -> String {
        format!(
            "<skills_instructions>\n## Skills\nA skill is a set of local instructions to follow that is stored in a `SKILL.md` file.\n### Available skills\n{lines}\n### How to use skills\n- Discovery: ...\n</skills_instructions>"
        )
    }

    #[test]
    fn parses_single_line_skill_entries() {
        let text = skills_block(
            "- qa-testing: Run the project's QA test plan against a live build (file: /home/user/skills/.system/qa-testing/SKILL.md)",
        );

        assert_eq!(
            parse_skills_instructions(&text),
            vec![SessionSkill {
                name: "qa-testing".to_string(),
                description: "Run the project's QA test plan against a live build".to_string(),
                path: "/home/user/skills/.system/qa-testing/SKILL.md".to_string(),
            }]
        );
    }

    #[test]
    fn parses_multi_line_descriptions_plugin_names_and_empty_descriptions() {
        let text = skills_block(
            "- apply-to-ramp: Guide a user through a Ramp application.\nUse when: \"apply to ramp\". (file: /home/user/.agents/skills/apply-to-ramp/SKILL.md)\n- ralph-loop:cancel-ralph: Cancel active Ralph Loop (file: r0/cancel-ralph/SKILL.md)\n- bare-skill: (file: /tmp/bare-skill/SKILL.md)",
        );

        assert_eq!(
            parse_skills_instructions(&text),
            vec![
                SessionSkill {
                    name: "apply-to-ramp".to_string(),
                    description:
                        "Guide a user through a Ramp application.\nUse when: \"apply to ramp\"."
                            .to_string(),
                    path: "/home/user/.agents/skills/apply-to-ramp/SKILL.md".to_string(),
                },
                SessionSkill {
                    name: "ralph-loop:cancel-ralph".to_string(),
                    description: "Cancel active Ralph Loop".to_string(),
                    path: "r0/cancel-ralph/SKILL.md".to_string(),
                },
                SessionSkill {
                    name: "bare-skill".to_string(),
                    description: String::new(),
                    path: "/tmp/bare-skill/SKILL.md".to_string(),
                },
            ]
        );
    }

    #[test]
    fn finds_skills_block_only_in_developer_messages() {
        let text = skills_block("- alpha: first (file: /tmp/alpha/SKILL.md)");
        let items = vec![
            ResponseItem::Message {
                encrypted_content: None,
                status: None,
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText { text: text.clone() }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::Message {
                encrypted_content: None,
                status: None,
                id: None,
                role: "developer".to_string(),
                content: vec![ContentItem::InputText { text }],
                phase: None,

                internal_chat_message_metadata_passthrough: None,
            },
        ];

        assert_eq!(parse_session_skills(&items).len(), 1);
        assert_eq!(parse_session_skills(&items[..1]).len(), 0);
    }

    #[test]
    fn ignores_blocks_without_available_skills_section() {
        let items = vec![ResponseItem::Message {
            encrypted_content: None,
            status: None,
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "<skills_instructions>\n- imagegen\n</skills_instructions>".to_string(),
            }],
            phase: None,

            internal_chat_message_metadata_passthrough: None,
        }];

        assert_eq!(parse_session_skills(&items), Vec::new());
    }
}
