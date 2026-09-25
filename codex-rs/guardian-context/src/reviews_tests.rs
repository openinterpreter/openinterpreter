//! Constructor boundaries preserve valid evidence and reject oversize sections.

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn previous_reviews_checks_count_and_size_without_rewriting_fragments() {
    let max_bytes = TruncationPolicy::Tokens(MAX_REVIEW_FRAGMENT_TOKENS).byte_budget();
    let fragment = "é".repeat(max_bytes / 2);
    let fragments = vec![fragment.clone(); MAX_PREVIOUS_REVIEWS];
    let reviews = PreviousReviews::try_from_fragments(fragments.clone()).unwrap();
    let ResponseItem::Message { content, .. } = reviews.into_message() else {
        panic!("expected a review message");
    };
    assert_eq!(
        &content[1..],
        fragments
            .into_iter()
            .map(|text| ContentItem::InputText { text })
            .collect::<Vec<_>>()
            .as_slice()
    );
    for invalid in [
        vec!["review".to_owned(); MAX_PREVIOUS_REVIEWS + 1],
        vec![format!("{fragment}a")],
    ] {
        assert_eq!(
            PreviousReviews::try_from_fragments(invalid),
            Err(SectionError::EvidenceLimitExceeded {
                section: "previous_reviews",
            })
        );
    }
}
