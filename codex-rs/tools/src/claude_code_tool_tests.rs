use super::*;
use crate::AdditionalProperties;
use crate::JsonSchemaPrimitiveType;
use crate::JsonSchemaType;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_BASH_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_GREP_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_READ_DESCRIPTION;
use crate::claude_code_tool_descriptions::CLAUDE_CODE_TODO_WRITE_DESCRIPTION;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn claude_code_agent_tool_matches_reference_schema_shape() {
    let ToolSpec::Function(ResponsesApiTool {
        name,
        description,
        parameters,
        output_schema,
        ..
    }) = create_claude_code_agent_tool()
    else {
        panic!("Agent should be a function tool");
    };

    assert_eq!(name, "Agent");
    assert!(description.contains("Launch a new agent to handle complex, multi-step tasks."));
    assert!(
        description
            .contains("When the agent is done, it will return a single message back to you.")
    );
    assert_eq!(
        parameters.schema_type,
        Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object))
    );

    let properties = parameters
        .properties
        .as_ref()
        .expect("Agent should use object params");
    assert_eq!(
        parameters.required.as_ref(),
        Some(&vec!["description".to_string(), "prompt".to_string()])
    );
    assert!(properties.contains_key("description"));
    assert!(properties.contains_key("prompt"));
    assert!(properties.contains_key("subagent_type"));
    assert!(properties.contains_key("model"));
    assert!(properties.contains_key("run_in_background"));
    assert!(properties.contains_key("isolation"));
    assert_eq!(output_schema, None);
}

#[test]
fn claude_code_core_tool_descriptions_match_reference_text() {
    let ToolSpec::Function(read_tool) = create_claude_code_read_tool() else {
        panic!("Read should be a function tool");
    };
    let ToolSpec::Function(grep_tool) = create_claude_code_grep_tool() else {
        panic!("Grep should be a function tool");
    };
    let ToolSpec::Function(todo_tool) = create_claude_code_todo_write_tool() else {
        panic!("TodoWrite should be a function tool");
    };
    let ToolSpec::Function(bash_tool) = create_claude_code_bash_tool() else {
        panic!("Bash should be a function tool");
    };

    assert_eq!(read_tool.description, CLAUDE_CODE_READ_DESCRIPTION);
    assert_eq!(grep_tool.description, CLAUDE_CODE_GREP_DESCRIPTION);
    assert_eq!(todo_tool.description, CLAUDE_CODE_TODO_WRITE_DESCRIPTION);
    assert_eq!(bash_tool.description, CLAUDE_CODE_BASH_DESCRIPTION);
}

#[test]
fn ask_user_question_tool_uses_claude_annotations_schema() {
    let ToolSpec::Function(tool) = create_claude_code_ask_user_question_tool() else {
        panic!("AskUserQuestion should be a function tool");
    };

    let annotations = tool
        .parameters
        .properties
        .as_ref()
        .and_then(|properties| properties.get("annotations"))
        .expect("annotations schema");

    assert_eq!(
        annotations.description.as_deref(),
        Some(
            "Optional per-question annotations from the user (e.g., notes on preview selections). Keyed by question text."
        )
    );
    assert_eq!(
        annotations.additional_properties,
        Some(AdditionalProperties::Schema(Box::new(JsonSchema::object(
            BTreeMap::from([
                (
                    "preview".to_string(),
                    JsonSchema::string(Some(
                        "The preview content of the selected option, if the question used previews."
                            .to_string(),
                    )),
                ),
                (
                    "notes".to_string(),
                    JsonSchema::string(Some(
                        "Free-text notes the user added to their selection.".to_string(),
                    )),
                ),
            ]),
            None,
            Some(false.into()),
        ))))
    );

    let answers = tool
        .parameters
        .properties
        .as_ref()
        .and_then(|properties| properties.get("answers"))
        .expect("answers schema");
    assert_eq!(
        answers.property_names,
        Some(Box::new(JsonSchema::string(None)))
    );

    let questions = tool
        .parameters
        .properties
        .as_ref()
        .and_then(|properties| properties.get("questions"))
        .expect("questions schema");
    assert_eq!(questions.min_items, Some(1));
    assert_eq!(questions.max_items, Some(4));
    let questions = questions.items.as_deref().expect("questions items schema");
    let question_properties = questions.properties.as_ref().expect("question properties");
    let options = question_properties.get("options").expect("options schema");
    assert_eq!(options.min_items, Some(2));
    assert_eq!(options.max_items, Some(4));
    let multi_select = question_properties
        .get("multiSelect")
        .expect("multiSelect schema");
    assert_eq!(multi_select.default_value, Some(false.into()));

    let metadata = tool
        .parameters
        .properties
        .as_ref()
        .and_then(|properties| properties.get("metadata"))
        .expect("metadata schema");
    assert_eq!(
        metadata.description.as_deref(),
        Some("Optional metadata for tracking and analytics purposes. Not displayed to user.")
    );
}
