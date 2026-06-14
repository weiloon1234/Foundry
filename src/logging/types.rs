use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    pub fn as_filter_directive(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HttpOutcomeClass {
    Informational,
    Success,
    Redirection,
    ClientError,
    ServerError,
}

impl HttpOutcomeClass {
    pub fn from_status(status: StatusCode) -> Self {
        match status.as_u16() / 100 {
            1 => Self::Informational,
            2 => Self::Success,
            3 => Self::Redirection,
            4 => Self::ClientError,
            _ => Self::ServerError,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthOutcome {
    Success,
    Unauthorized,
    Forbidden,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    Enqueued,
    Leased,
    Started,
    Succeeded,
    Retried,
    ExpiredLeaseRequeued,
    DeadLettered,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketConnectionState {
    Opened,
    Closed,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeBackendKind {
    Redis,
    Memory,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerLeadershipState {
    Acquired,
    Lost,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeState {
    Healthy,
    Unhealthy,
}

impl ProbeState {
    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }
}
