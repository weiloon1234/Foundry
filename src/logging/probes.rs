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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessProbeDescriptor {
    pub id: ProbeId,
    pub built_in: bool,
}

#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    foundry_macros::TS,
    foundry_macros::ApiSchema,
)]
pub struct ProbeResult {
    pub id: ProbeId,
    pub state: ProbeState,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
}

impl ts_rs::TS for ProbeResult {
    type WithoutGenerics = Self;

    fn name() -> String {
        "ProbeResult".to_string()
    }

    fn decl() -> String {
        "type ProbeResult = { id: string, state: ProbeState, message?: string | null, };"
            .to_string()
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn inline() -> String {
        "{ id: string, state: ProbeState, message?: string | null, }".to_string()
    }

    fn inline_flattened() -> String {
        Self::inline()
    }

    fn visit_dependencies(visitor: &mut impl ts_rs::TypeVisitor)
    where
        Self: 'static,
    {
        visitor.visit::<ProbeState>();
    }

    fn output_path() -> Option<&'static std::path::Path> {
        Some(std::path::Path::new("ProbeResult.ts"))
    }
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

impl ReadinessRegistry {
    pub(crate) fn descriptors(&self) -> Vec<ReadinessProbeDescriptor> {
        let mut descriptors = self
            .checks
            .iter()
            .map(|check| ReadinessProbeDescriptor {
                id: check.id.clone(),
                built_in: readiness_probe_is_built_in(&check.id),
            })
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        descriptors
    }
}

pub(crate) struct RegisteredReadinessCheck {
    pub(crate) id: ProbeId,
    pub(crate) check: Arc<dyn ReadinessCheck>,
}

fn readiness_probe_is_built_in(id: &ProbeId) -> bool {
    id == &FRAMEWORK_BOOTSTRAP_PROBE || id == &RUNTIME_BACKEND_PROBE || id == &REDIS_PING_PROBE
}
