use pretty_assertions::assert_eq;
use serde_json::json;
use ts_rs::TS;

use super::ExtensionItem;
use super::image_generation::ImageGenerationItem;
use super::sleep::SleepItem;
use super::web_search::WebSearchAction;
use super::web_search::WebSearchItem;

fn completed_image_generation_item() -> ExtensionItem {
    ExtensionItem::ImageGeneration(ImageGenerationItem {
        id: "image-1".to_string(),
        status: "completed".to_string(),
        revised_prompt: Some("A blue square".to_string()),
        result: "cG5n".to_string(),
        transparent_background: None,
        failure: None,
        saved_path: None,
        imagegen_request_id: None,
        generation_id: None,
    })
}

#[test]
fn image_generation_item_preserves_stable_wire_shape() {
    let item = completed_image_generation_item();
    let value = serde_json::to_value(&item).expect("serialize extension item");

    assert_eq!(
        value,
        json!({
            "kind": "image_gen.generation",
            "id": "image-1",
            "status": "completed",
            "revisedPrompt": "A blue square",
            "result": "cG5n",
            "transparentBackground": null,
            "failure": null,
        })
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(value).expect("deserialize extension item"),
        item
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(json!({
            "kind": "image_gen.generation",
            "id": "image-1",
            "status": "completed",
            "revisedPrompt": "A blue square",
            "result": "cG5n",
        }))
        .expect("deserialize legacy image-generation item without transparency metadata"),
        item
    );
}

#[test]
fn image_generation_item_preserves_authoritative_transparency() {
    let ExtensionItem::ImageGeneration(mut image) = completed_image_generation_item() else {
        panic!("expected image-generation item");
    };
    image.transparent_background = Some(true);
    let item = ExtensionItem::ImageGeneration(image);
    let value = serde_json::to_value(&item).expect("serialize extension item");

    assert_eq!(
        value,
        json!({
            "kind": "image_gen.generation",
            "id": "image-1",
            "status": "completed",
            "revisedPrompt": "A blue square",
            "result": "cG5n",
            "transparentBackground": true,
            "failure": null,
        })
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(value).expect("deserialize extension item"),
        item
    );
}

#[test]
fn image_generation_transparency_is_optional_in_typescript() {
    assert!(
        ImageGenerationItem::inline().contains("transparentBackground?: boolean"),
        "image-generation transparency must remain optional for existing TypeScript clients"
    );
}

#[test]
fn image_generation_request_id_stays_internal() {
    let ExtensionItem::ImageGeneration(mut image) = completed_image_generation_item() else {
        panic!("expected image-generation item");
    };
    image.imagegen_request_id = Some("req-imagegen-123".to_string());
    image.generation_id = Some("gen-image-123".to_string());
    let item = ExtensionItem::ImageGeneration(image);
    let value = serde_json::to_value(&item).expect("serialize extension item");

    assert!(value.get("imagegenRequestId").is_none());
    assert!(value.get("generationId").is_none());
    let ExtensionItem::ImageGeneration(round_tripped) =
        serde_json::from_value::<ExtensionItem>(value).expect("deserialize extension item")
    else {
        panic!("expected image-generation item");
    };
    assert_eq!(round_tripped.imagegen_request_id, None);
    assert_eq!(round_tripped.generation_id, None);
    assert!(!ImageGenerationItem::inline().contains("imagegenRequestId"));
    assert!(!ImageGenerationItem::inline().contains("generationId"));
}

#[test]
fn web_search_item_preserves_stable_wire_shape() {
    let item = ExtensionItem::WebSearch(WebSearchItem {
        id: "search-1".to_string(),
        query: "docs".to_string(),
        action: Some(WebSearchAction::Search {
            query: Some("docs".to_string()),
            queries: None,
        }),
        results: None,
    });
    let value = serde_json::to_value(&item).expect("serialize extension item");

    assert_eq!(
        value,
        json!({
            "kind": "web.search",
            "id": "search-1",
            "query": "docs",
            "action": {
                "type": "search",
                "query": "docs",
                "queries": null,
            },
            "results": null,
        })
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(value).expect("deserialize extension item"),
        item
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(json!({
            "kind": "web.search",
            "id": "search-1",
            "query": "docs",
            "action": {
                "type": "search",
                "query": "docs",
                "queries": null,
            },
        }))
        .expect("deserialize legacy extension item without results"),
        item
    );
}

#[test]
fn sleep_item_preserves_stable_wire_shape() {
    let item = ExtensionItem::Sleep(SleepItem {
        id: "sleep-1".to_string(),
        duration_ms: 1_000,
    });
    let value = serde_json::to_value(&item).expect("serialize extension item");

    assert_eq!(
        value,
        json!({
            "kind": "clock.sleep",
            "id": "sleep-1",
            "durationMs": 1_000,
        })
    );
    assert_eq!(
        serde_json::from_value::<ExtensionItem>(value).expect("deserialize extension item"),
        item
    );
}

#[test]
fn unknown_extension_kind_is_rejected() {
    let value = json!({
        "kind": "image_gen.unknown",
        "id": "image-1",
    });

    assert!(serde_json::from_value::<ExtensionItem>(value).is_err());
}

#[test]
fn malformed_known_extension_payload_is_rejected() {
    let value = json!({
        "kind": "image_gen.generation",
        "id": "image-1",
        "status": "completed",
    });

    assert!(serde_json::from_value::<ExtensionItem>(value).is_err());
}
