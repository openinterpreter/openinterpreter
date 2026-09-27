/// Host-provided metrics capability for extension-owned behavior.
///
/// Implementations are expected to attach the host's session attribution before
/// forwarding samples to the configured metrics backend.
pub trait ExtensionMetrics: Send + Sync {
    /// Increments a counter with optional extension-provided tags.
    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]);

    /// Records one histogram sample with optional extension-provided tags.
    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]);

    /// Records a histogram with explicit buckets, preserving host attribution.
    /// All callers of the same metric name must use the same boundaries.
    fn histogram_with_boundaries(
        &self,
        name: &str,
        value: i64,
        boundaries: &[f64],
        tags: &[(&str, &str)],
    );
}
