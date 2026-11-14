use std::sync::Mutex;

use codex_extension_api::ExtensionMetrics;
use pretty_assertions::assert_eq;

use super::*;

#[derive(Debug, PartialEq)]
struct RecordedHistogram {
    name: String,
    value: i64,
    boundaries: Vec<f64>,
    tags: Vec<(String, String)>,
}

#[derive(Default)]
struct RecordingMetrics {
    samples: Mutex<Vec<RecordedHistogram>>,
}

impl ExtensionMetrics for RecordingMetrics {
    fn histogram_with_boundaries(
        &self,
        name: &str,
        value: i64,
        boundaries: &[f64],
        tags: &[(&str, &str)],
    ) {
        self.samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(RecordedHistogram {
                name: name.to_string(),
                value,
                boundaries: boundaries.to_vec(),
                tags: tags
                    .iter()
                    .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                    .collect(),
            });
    }

    fn counter(&self, name: &str, _inc: i64, _tags: &[(&str, &str)]) {
        panic!("unexpected counter: {name}");
    }

    fn histogram(&self, name: &str, _value: i64, _tags: &[(&str, &str)]) {
        panic!("unexpected histogram without context boundaries: {name}");
    }
}

#[test]
fn records_core_equivalent_catalog_render_metrics_with_surface() {
    let metrics = RecordingMetrics::default();
    let report = SkillRenderReport {
        total_count: 5,
        included_count: 3,
        omitted_count: 2,
        truncated_description_chars: 700,
        truncated_description_count: 4,
    };

    record_catalog_render(
        Some(&metrics),
        CatalogSurface::TurnInput,
        SkillMetadataBudget::Tokens(400),
        &report,
    );

    assert_eq!(
        *metrics
            .samples
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![
            RecordedHistogram {
                name: THREAD_SKILLS_ENABLED_TOTAL_METRIC.to_string(),
                value: 5,
                boundaries: THREAD_SKILLS_COUNT_METRIC_BUCKETS.to_vec(),
                tags: vec![("catalog_surface".to_string(), "turn_input".to_string())],
            },
            RecordedHistogram {
                name: THREAD_SKILLS_KEPT_TOTAL_METRIC.to_string(),
                value: 3,
                boundaries: THREAD_SKILLS_COUNT_METRIC_BUCKETS.to_vec(),
                tags: vec![("catalog_surface".to_string(), "turn_input".to_string())],
            },
            RecordedHistogram {
                name: THREAD_SKILLS_TRUNCATED_METRIC.to_string(),
                value: 1,
                boundaries: THREAD_SKILLS_TRUNCATED_BUCKETS.to_vec(),
                tags: vec![("catalog_surface".to_string(), "turn_input".to_string())],
            },
            RecordedHistogram {
                name: THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC.to_string(),
                value: 700,
                boundaries: THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_BUCKETS.to_vec(),
                tags: vec![("catalog_surface".to_string(), "turn_input".to_string())],
            },
        ]
    );
}
