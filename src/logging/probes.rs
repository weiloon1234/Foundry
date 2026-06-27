use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::types::ProbeState;
use crate::foundation::{AppContext, Error, Result};
use crate::support::sync::lock_unpoisoned;
use crate::support::ProbeId;

pub const FRAMEWORK_BOOTSTRAP_PROBE: ProbeId = ProbeId::new("foundry.bootstrap");
pub const RUNTIME_BACKEND_PROBE: ProbeId = ProbeId::new("foundry.runtime_backend");
pub const REDIS_PING_PROBE: ProbeId = ProbeId::new("foundry.redis_ping");

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct ProbeResult {
    pub id: ProbeId,
    pub state: ProbeState,
    #[serde(default)]
    pub message: Option<String>,
}

impl ProbeResult {
    pub fn healthy<I>(id: I) -> Self
    where
        I: Into<ProbeId>,
    {
        Self {
            id: id.into(),
            state: ProbeState::Healthy,
            message: None,
        }
    }

    pub fn unhealthy<I>(id: I, message: impl Into<String>) -> Self
    where
        I: Into<ProbeId>,
    {
        Self {
            id: id.into(),
            state: ProbeState::Unhealthy,
            message: Some(message.into()),
        }
    }
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct LivenessReport {
    pub state: ProbeState,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    ts_rs::TS,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct ReadinessReport {
    pub state: ProbeState,
    pub probes: Vec<ProbeResult>,
}

#[async_trait]
pub trait ReadinessCheck: Send + Sync + 'static {
    async fn run(&self, app: &AppContext) -> Result<ProbeResult>;
}

#[async_trait]
impl<F, Fut> ReadinessCheck for F
where
    F: Fn(&AppContext) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<ProbeResult>> + Send,
{
    async fn run(&self, app: &AppContext) -> Result<ProbeResult> {
        (self)(app).await
    }
}

pub(crate) type ReadinessRegistryHandle = Arc<Mutex<ReadinessRegistryBuilder>>;

#[derive(Default)]
pub(crate) struct ReadinessRegistryBuilder {
    pub(crate) checks: Vec<RegisteredReadinessCheck>,
    ids: HashSet<ProbeId>,
}

impl ReadinessRegistryBuilder {
    pub(crate) fn shared() -> ReadinessRegistryHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub(crate) fn register_arc<I>(&mut self, id: I, check: Arc<dyn ReadinessCheck>) -> Result<()>
    where
        I: Into<ProbeId>,
    {
        let id = id.into();
        if !self.ids.insert(id.clone()) {
            return Err(Error::message(format!(
                "readiness check `{id}` already registered"
            )));
        }

        self.checks.push(RegisteredReadinessCheck { id, check });
        Ok(())
    }

    pub(crate) fn freeze_shared(handle: ReadinessRegistryHandle) -> ReadinessRegistry {
        let mut builder = lock_unpoisoned(&handle, "readiness registry");
        ReadinessRegistry {
            checks: std::mem::take(&mut builder.checks),
        }
    }
}

pub(crate) struct ReadinessRegistry {
    pub(crate) checks: Vec<RegisteredReadinessCheck>,
}

pub(crate) struct RegisteredReadinessCheck {
    pub(crate) id: ProbeId,
    pub(crate) check: Arc<dyn ReadinessCheck>,
}
