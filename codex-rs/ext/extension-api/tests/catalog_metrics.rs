use std::sync::Mutex;

use codex_extension_api::ExtensionMetrics;
use codex_extension_api::record_catalog_entries;

#[derive(Default)]
struct RecordedMetrics(Mutex<Vec<(String, i64, Vec<(String, String)>)>>);

impl ExtensionMetrics for RecordedMetrics {
    fn counter(&self, _name: &str, _inc: i64, _tags: &[(&str, &str)]) {
        panic!("catalog entries must be a histogram");
    }

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        self.0.lock().unwrap().push((
            name.to_owned(),
            value,
            tags.iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        ));
    }

    fn histogram_with_boundaries(
        &self,
        _name: &str,
        _value: i64,
        _boundaries: &[f64],
        _tags: &[(&str, &str)],
    ) {
        panic!("catalog entries must use the ordinary histogram");
    }
}

#[test]
fn catalog_metrics_record_publication_status_and_skip_absent_host_metrics() {
    record_catalog_entries(None, "catalog.entries", "cloud", 7, false, 0);
    let metrics = RecordedMetrics::default();
    for (stage, count, partial, warnings) in [
        ("cloud", 7, false, 0),
        ("executor", 3, true, 0),
        ("executor", 2, false, 1),
    ] {
        record_catalog_entries(
            Some(&metrics),
            "catalog.entries",
            stage,
            count,
            partial,
            warnings,
        );
    }
    assert_eq!(
        *metrics.0.lock().unwrap(),
        vec![
            sample("cloud", "complete", 7),
            sample("executor", "partial", 3),
            sample("executor", "warning", 2),
        ]
    );
}

fn sample(stage: &str, status: &str, value: i64) -> (String, i64, Vec<(String, String)>) {
    (
        "catalog.entries".to_owned(),
        value,
        vec![
            ("stage".to_owned(), stage.to_owned()),
            ("status".to_owned(), status.to_owned()),
        ],
    )
}
