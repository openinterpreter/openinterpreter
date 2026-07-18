use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::SkillScope;
use serde::Deserialize;

#[derive(Deserialize)]
struct KimiSkillArgs {
    skill: String,
    #[serde(default)]
    args: Option<String>,
}

pub(super) async fn handle(
    invocation: ToolInvocation,
) -> Result<Box<dyn ToolOutput>, FunctionCallError> {
    let ToolPayload::Function { arguments } = &invocation.payload else {
        return Err(FunctionCallError::RespondToModel(
            "Kimi Code Skill received unsupported payload".to_string(),
        ));
    };
    let input: KimiSkillArgs = serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse Skill arguments: {err}"))
    })?;
    let args = input.args.as_deref().unwrap_or_default();
    let skill_name = input.skill.as_str();
    let builtin_contents = match skill_name {
        "check-kimi-code-docs" => Some(include_str!("kimi_code_skills/check-kimi-code-docs.md")),
        "update-config" => Some(include_str!("kimi_code_skills/update-config.md")),
        "write-goal" => Some(include_str!("kimi_code_skills/write-goal.md")),
        _ => None,
    };
    let (contents, source, directory) = if let Some(contents) = builtin_contents {
        (
            contents.to_string(),
            "builtin",
            format!("builtin://{skill_name}"),
        )
    } else {
        let outcome = invocation.turn.turn_skills.snapshot.outcome();
        let Some(skill) = outcome
            .skills
            .iter()
            .find(|candidate| candidate.name == input.skill && outcome.is_skill_enabled(candidate))
            .cloned()
        else {
            return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                format!(
                    "Skill \"{skill}\" not found in the current skill listing.",
                    skill = input.skill
                ),
                /*success*/ Some(false),
            )));
        };
        let contents = match invocation
            .turn
            .turn_skills
            .snapshot
            .read_skill_text(&skill)
            .await
        {
            Ok(contents) => contents,
            Err(err) => {
                return Ok(boxed_tool_output(FunctionToolOutput::from_text(
                    format!(
                        "Failed to load Skill \"{skill}\": {err}",
                        skill = input.skill
                    ),
                    /*success*/ Some(false),
                )));
            }
        };
        let source = kimi_skill_source(skill.scope);
        let directory = skill
            .path_to_skills_md
            .parent()
            .unwrap_or_else(|| skill.path_to_skills_md.clone())
            .to_string_lossy()
            .into_owned();
        (contents, source, directory)
    };
    let body = expand_skill_body(&contents, args);
    let message = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!(
                "Skill tool loaded instructions for this request. Follow them.\n\n<kimi-skill-loaded name=\"{}\" trigger=\"model-tool\" source=\"{source}\" dir=\"{}\" args=\"{}\">\n{body}\n</kimi-skill-loaded>",
                escape_xml_attribute(&input.skill),
                escape_xml_attribute(&directory),
                escape_xml_attribute(args),
            ),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    };
    invocation
        .session
        .record_conversation_items(invocation.turn.as_ref(), &[message])
        .await;

    Ok(boxed_tool_output(FunctionToolOutput::from_text(
        format!(
            "Skill \"{name}\" loaded inline. Follow its instructions.",
            name = input.skill
        ),
        /*success*/ Some(true),
    )))
}

fn kimi_skill_source(scope: SkillScope) -> &'static str {
    match scope {
        SkillScope::Repo => "project",
        SkillScope::User => "user",
        SkillScope::System | SkillScope::Admin => "extra",
    }
}

fn expand_skill_body(contents: &str, args: &str) -> String {
    let contents = contents.replace("\r\n", "\n");
    let body = strip_frontmatter(&contents).trim();
    if args.is_empty() {
        return body.to_string();
    }

    let escaped_args = escape_xml_tags(args);
    if body.contains("$ARGUMENTS") {
        body.replace("$ARGUMENTS", &escaped_args)
    } else {
        format!("{body}\n\nARGUMENTS: {escaped_args}")
    }
}

fn strip_frontmatter(contents: &str) -> &str {
    contents
        .strip_prefix("---\n")
        .and_then(|contents| contents.split_once("\n---\n").map(|(_, body)| body))
        .or_else(|| {
            contents
                .strip_prefix("---\r\n")
                .and_then(|contents| contents.split_once("\r\n---\r\n").map(|(_, body)| body))
        })
        .unwrap_or(contents)
}

fn escape_xml_attribute(value: &str) -> String {
    escape_xml_tags(value)
        .replace('&', "&amp;")
        .replace('"', "&quot;")
}

fn escape_xml_tags(value: &str) -> String {
    value.replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_body_without_frontmatter_or_empty_arguments_suffix() {
        let contents = "---\nname: qa-testing\ndescription: Test apps\n---\n\n# QA testing\n\nUse cua-driver.\n";

        assert_eq!(
            expand_skill_body(contents, ""),
            "# QA testing\n\nUse cua-driver."
        );
    }

    #[test]
    fn normalizes_windows_line_endings() {
        let contents =
            "---\r\nname: qa-testing\r\ndescription: Test apps\r\n---\r\n\r\n# QA testing\r\n";

        assert_eq!(expand_skill_body(contents, ""), "# QA testing");
    }

    #[test]
    fn appends_and_escapes_arguments_when_body_has_no_placeholder() {
        assert_eq!(
            expand_skill_body("# Review", "<raw \"value\">"),
            "# Review\n\nARGUMENTS: &lt;raw \"value\"&gt;"
        );
    }

    #[test]
    fn replaces_arguments_placeholder() {
        assert_eq!(expand_skill_body("Raw: $ARGUMENTS", "a & b"), "Raw: a & b");
    }

    #[test]
    fn maps_host_skill_scopes_to_kimi_sources() {
        assert_eq!(kimi_skill_source(SkillScope::Repo), "project");
        assert_eq!(kimi_skill_source(SkillScope::User), "user");
        assert_eq!(kimi_skill_source(SkillScope::System), "extra");
        assert_eq!(kimi_skill_source(SkillScope::Admin), "extra");
    }

    #[test]
    fn provider_default_skill_assets_match_the_expected_catalog() {
        for (name, contents) in [
            (
                "check-kimi-code-docs",
                include_str!("kimi_code_skills/check-kimi-code-docs.md"),
            ),
            (
                "update-config",
                include_str!("kimi_code_skills/update-config.md"),
            ),
            ("write-goal", include_str!("kimi_code_skills/write-goal.md")),
        ] {
            let contents = contents.replace("\r\n", "\n");
            assert!(contents.starts_with("---\nname: "));
            assert!(contents.contains(&format!("name: {name}\n")));
            assert!(!expand_skill_body(contents, "").is_empty());
        }
    }
}
