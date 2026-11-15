//! Consent follows the actual counter destination across exporter replacements.

use super::*;
use opentelemetry_sdk::metrics::data::AggregatedMetrics;
use opentelemetry_sdk::metrics::data::MetricData;
use opentelemetry_sdk::metrics::data::ScopeMetrics;
use opentelemetry_sdk::metrics::data::SumDataPoint;
use pretty_assertions::assert_eq;

#[test]
fn counter_consent_uses_the_active_exporter() -> Result<()> {
    let mut builtin = MetricsClient::new(
        MetricsConfig::in_memory("test", "test", "1", Default::default()).with_runtime_reader(),
    )?;
    // Use the existing in-memory exporter to observe the built-in consent policy without
    // contacting Statsig (the real Statsig exporter is disabled in debug builds).
    Arc::get_mut(&mut builtin.inner)
        .expect("unique provider")
        .requires_analytics_consent = true;
    let custom = MetricsClient::new(
        MetricsConfig::in_memory("test", "test", "1", Default::default()).with_runtime_reader(),
    )?;
    let active = Arc::new(RwLock::new(Arc::clone(&builtin.inner)));
    let metrics = MetricsClient {
        inner: Arc::clone(&builtin.inner),
        active: Some(Arc::clone(&active)),
    };

    // A config reload alone does not replace the installed built-in exporter.
    for analytics_enabled in [false, true] {
        metrics.counter_with_analytics_consent(
            "test.observations",
            /*inc*/ 1,
            &[],
            analytics_enabled,
        )?;
    }
    *active.write().unwrap() = Arc::clone(&custom.inner);
    metrics.counter_with_analytics_consent(
        "test.observations",
        /*inc*/ 1,
        &[],
        /*analytics_enabled*/ false,
    )?;

    let mut counts = Vec::new();
    for destination in [&builtin, &custom] {
        let snapshot = destination.snapshot()?;
        for metric in snapshot.scope_metrics().flat_map(ScopeMetrics::metrics) {
            if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() {
                counts.extend(sum.data_points().map(SumDataPoint::value));
            }
        }
    }
    assert_eq!(counts, vec![1, 1]);
    Ok(())
}
