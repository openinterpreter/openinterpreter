use codex_protocol::models::AudioReference;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ResponseInputItem;
use pretty_assertions::assert_eq;

use super::response_input_to_code_mode_result;

#[test]
fn code_mode_text_preserves_file_backed_image_id() {
    let response = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputImage {
            image: ImageReference::File {
                file_id: "file_123".to_string(),
            },
            detail: None,
        }],
        phase: None,
    };

    assert_eq!(
        response_input_to_code_mode_result(response),
        serde_json::json!("file_123")
    );
}

#[test]
fn code_mode_text_preserves_file_backed_audio_id() {
    let response = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputAudio {
            audio: AudioReference::File {
                file_id: "audio_file_123".to_string(),
            },
        }],
        phase: None,
    };
    assert_eq!(
        response_input_to_code_mode_result(response),
        serde_json::json!("audio_file_123")
    );
}
