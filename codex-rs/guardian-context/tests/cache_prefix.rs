//! Exercises cache-prefix stability through the public collection and composition API.

use codex_context_fragments::ContextualUserFragment;
use codex_guardian_context::ActionPresentation;
use codex_guardian_context::ContextBudgeting;
use codex_guardian_context::ContextPresentation;
use codex_guardian_context::ContextProfile;
use codex_guardian_context::ContextTarget;
use codex_guardian_context::PlannedAction;
use codex_guardian_context::PlannedActionKind;
use codex_guardian_context::PreviousReviews;
use codex_guardian_context::SectionHistory;
use codex_guardian_context::SectionInput;
use codex_guardian_context::TrustedSkills;
use codex_guardian_context::TrustedTool;
use codex_guardian_context::TrustedToolSource;
use codex_guardian_context::default_registry;
use codex_guardian_context::prepare_context;
use codex_history::RetainedContext;
use codex_history::RetainedInputSource;
use codex_history::RetainedUserMessage;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol_test_support::uuid_v7_item_id_generator_for_test;
use pretty_assertions::assert_eq;
use std::borrow::Cow;

struct History {
    items: Vec<ResponseItem>,
    retained: Option<RetainedContext>,
}

impl SectionHistory for History {
    fn items(&self) -> Box<dyn Iterator<Item = &ResponseItem> + Send + '_> {
        Box::new(self.items.iter())
    }

    fn retained_context(&self) -> Option<&RetainedContext> {
        self.retained.as_ref()
    }
}

fn user_message(texts: Vec<String>) -> ResponseItem {
    ResponseItem::Message {
        status: None,
        encrypted_content: None,
        id: None,
        role: "user".to_owned(),
        content: texts
            .into_iter()
            .map(|text| ContentItem::InputText { text })
            .collect(),
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn changing_attestations_preserves_history_before_the_current_action() {
    let instruction = "Inspect staging only. Do not publish.";
    let mut retained = RetainedContext::default();
    retained.record_user_message(
        RetainedUserMessage {
            turn_id: "turn-1".to_owned(),
            message_id: None,
            text: instruction.to_owned(),
            complete: true,
            origin: codex_history::UserInputOrigin::User,
            phase: None,
        },
        RetainedInputSource::Local(None),
        &uuid_v7_item_id_generator_for_test(),
    );
    // Legacy review history and Felix's retained, thread-owned history use the
    // same composer. Neither may put changing attestations before that history.
    for retained in [None, Some(retained)] {
        let history = History {
            items: vec![user_message(vec![instruction.to_owned()])],
            retained,
        };
        let profile = ContextProfile::asynchronous();
        let mut previous_prefix = None;
        for generation in 0..2 {
            let reviews =
                PreviousReviews::try_from_fragments(vec![codex_guardian_context::PreviousReview {
                    id: uuid_v7_item_id_generator_for_test()("review"),
                    fragment: format!("Host-attested decision {generation}: denied."),
                }])
                .unwrap();
            let tool = TrustedTool {
                server: format!("server-{generation}"),
                connector_id: None,
                source: TrustedToolSource::UserConfiguration("user configuration".to_owned()),
            };
            let skills = TrustedSkills {
                paths: vec![format!("/skills/skill-{generation}/SKILL.md")],
            };
            let action = PlannedAction {
                tool_descriptions: None,
                kind: PlannedActionKind::Command {
                    json: format!(r#"{{"tool":"inspect","path":"file-{generation}"}}"#),
                },
                reason: None,
            };
            let collected = default_registry()
                .prepare(&SectionInput {
                    target: ContextTarget::Async,
                    history: &history,
                    transcript: &profile.transcript,
                    is_selected_exec_output: None,
                    root_conversation: &[],
                    root_conversation_candidates: None,
                    trusted_user_answers: &[],
                    planned_action: Some(&action),
                    permissions: None,
                    previous_reviews: Some(&reviews),
                    trusted_tool: Some(&tool),
                    trusted_skill_paths: &skills.paths,
                    images: None,
                    node_repl: None,
                })
                .unwrap();
            let messages = prepare_context(
                Cow::Owned(collected),
                &profile,
                /*entry_number_offset*/ 0,
                ContextPresentation::Async,
                ContextBudgeting::Fixed,
                &[],
            )
            .unwrap()
            .into_messages();
            let (prefix, suffix) = messages.split_first().expect("history prefix");
            let ResponseItem::Message { role, content, .. } = prefix else {
                panic!("history remains untrusted user evidence");
            };
            assert_eq!(role, "user");
            assert!(content.iter().any(|item| matches!(
                item,
                ContentItem::InputText { text } if text == ">>> TRANSCRIPT END\n\n"
            )));
            if let Some(previous) = previous_prefix.replace(prefix.clone()) {
                assert_eq!(prefix, &previous);
            }
            assert_eq!(
                &suffix[..3],
                &[
                    reviews.into_annotated_message().item,
                    ContextualUserFragment::into(tool),
                    ContextualUserFragment::into(skills),
                ]
            );
            assert_eq!(suffix.len(), 4);
            let ResponseItem::Message { role, content, .. } = &suffix[3] else {
                panic!("retained instructions and action remain untrusted user evidence");
            };
            assert_eq!(role, "user");
            let action_content = action
                .render(ActionPresentation::Async)
                .into_iter()
                .map(|text| ContentItem::InputText { text })
                .collect::<Vec<_>>();
            assert!(content.ends_with(&action_content));
            if history.retained.is_some() {
                assert!(content[..content.len() - action_content.len()].iter().any(|item| {
                    matches!(item, ContentItem::InputText { text } if text.contains(instruction))
                }));
            } else {
                assert_eq!(content, &action_content);
            }
        }
    }
}
