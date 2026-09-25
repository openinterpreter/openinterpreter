use std::sync::Arc;

use codex_extension_api::ExtensionMetrics;
use codex_otel::SessionTelemetry;

struct SessionTelemetryExtensionMetrics {
    session_telemetry: SessionTelemetry,
}

impl ExtensionMetrics for SessionTelemetryExtensionMetrics {
    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.counter(name, inc, tags);
    }

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.histogram(name, value, tags);
    }

    fn histogram_with_boundaries(
        &self,
        name: &str,
        value: i64,
        boundaries: &[f64],
        tags: &[(&str, &str)],
    ) {
        self.session_telemetry
            .histogram_with_boundaries(name, value, boundaries, tags);
    }
}

pub(crate) fn from_session_telemetry(
    session_telemetry: SessionTelemetry,
) -> Arc<dyn ExtensionMetrics> {
    Arc::new(SessionTelemetryExtensionMetrics { session_telemetry })
}
