// The health probe the deploy runbook waits on before advancing a stage.
pub fn is_healthy(status: &Status) -> bool {
    status.error_rate < 0.02 && status.p99_latency_ms < 250 && status.ready
}
