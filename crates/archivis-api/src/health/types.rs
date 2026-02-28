use serde::Serialize;

/// Response body for liveness and readiness probes.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<HealthChecks>,
}

/// Per-subsystem health check results (returned by readiness probe).
#[derive(Debug, Serialize)]
pub struct HealthChecks {
    pub database: &'static str,
}
