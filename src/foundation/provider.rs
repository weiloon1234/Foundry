use std::sync::Arc;

use async_trait::async_trait;

use crate::auth::{
    ActorHydrator, ActorHydratorRegistryHandle, Authenticatable, AuthenticatableRegistryHandle,
    BearerAuthenticator, GuardRegistryHandle, Policy, PolicyRegistryHandle,
};
use crate::config::ConfigRepository;
use crate::database::{MigrationFile, MigrationRegistryHandle, SeederFile, SeederRegistryHandle};
use crate::datatable::registry::{DatatableRegistryBuilder, DatatableRegistryHandle};
use crate::email::{EmailDriverFactory, EmailDriverRegistryHandle};
use crate::events::{Event, EventListener, EventRegistryHandle};
use crate::foundation::{AppContext, Container, Result};
use crate::jobs::{Job, JobMiddleware, JobMiddlewareRegistryHandle, JobRegistryHandle};
use crate::logging::{ReadinessCheck, ReadinessRegistryHandle};
use crate::notifications::{
    NotificationChannel, NotificationChannelRegistryBuilder, NotificationChannelRegistryHandle,
};
use crate::storage::{StorageDriverFactory, StorageDriverRegistryHandle};
use crate::support::sync::lock_unpoisoned;
use crate::support::{GuardId, MigrationId, PolicyId, ProbeId, SeederId};
use crate::validation::RuleRegistry;

#[derive(Clone)]
pub(crate) struct RegistryHub {
    pub(crate) event: EventRegistryHandle,
    pub(crate) job: JobRegistryHandle,
    pub(crate) job_middleware: JobMiddlewareRegistryHandle,
    pub(crate) migration: MigrationRegistryHandle,
    pub(crate) seeder: SeederRegistryHandle,
    pub(crate) guard: GuardRegistryHandle,
    pub(crate) actor_hydrator: ActorHydratorRegistryHandle,
    pub(crate) policy: PolicyRegistryHandle,
    pub(crate) authenticatable: AuthenticatableRegistryHandle,
    pub(crate) readiness: ReadinessRegistryHandle,
    pub(crate) storage_driver: StorageDriverRegistryHandle,
    pub(crate) email_driver: EmailDriverRegistryHandle,
    pub(crate) notification_channel: NotificationChannelRegistryHandle,
    pub(crate) datatable: DatatableRegistryHandle,
}

impl RegistryHub {
    pub(crate) fn new() -> Self {
        Self {
            event: crate::events::EventRegistryBuilder::shared(),
            job: crate::jobs::JobRegistryBuilder::shared(),
            job_middleware: crate::jobs::JobMiddlewareRegistryBuilder::shared(),
            migration: crate::database::MigrationRegistryBuilder::shared(),
            seeder: crate::database::SeederRegistryBuilder::shared(),
            guard: crate::auth::GuardRegistryBuilder::shared(),
            actor_hydrator: crate::auth::ActorHydratorRegistryBuilder::shared(),
            policy: crate::auth::PolicyRegistryBuilder::shared(),
            authenticatable: crate::auth::AuthenticatableRegistryBuilder::shared(),
            readiness: crate::logging::ReadinessRegistryBuilder::shared(),
            storage_driver: crate::storage::StorageDriverRegistryBuilder::shared(),
            email_driver: crate::email::EmailDriverRegistryBuilder::shared(),
            notification_channel: NotificationChannelRegistryBuilder::shared(),
            datatable: DatatableRegistryBuilder::shared(),
        }
    }
}

#[derive(Clone)]
pub struct ServiceRegistrar {
    container: Container,
    config: ConfigRepository,
    rules: RuleRegistry,
    registries: RegistryHub,
}

impl ServiceRegistrar {
    pub(crate) fn new(
        container: Container,
        config: ConfigRepository,
        rules: RuleRegistry,
        registries: RegistryHub,
    ) -> Self {
        Self {
            container,
            config,
            rules,
            registries,
        }
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    pub fn config(&self) -> &ConfigRepository {
        &self.config
    }

    pub fn singleton<T>(&self, value: T) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        self.container.singleton(value)
    }

    pub fn singleton_arc<T>(&self, value: Arc<T>) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        self.container.singleton_arc(value)
    }

    pub fn factory<T, F>(&self, factory: F) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn(&Container, &AppContext) -> Result<T> + Send + Sync + 'static,
    {
        let config = self.config.clone();
        let rules = self.rules.clone();

        self.container.factory(move |container| {
            let app = AppContext::new(container.clone(), config.clone(), rules.clone())?;
            factory(container, &app)
        })
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.container.resolve::<T>()
    }

    pub fn listen_event<E, L>(&self, listener: L) -> Result<()>
    where
        E: Event,
        L: EventListener<E>,
    {
        lock_unpoisoned(&self.registries.event, "event registry").listen::<E, L>(listener)
    }

    pub fn register_job<J>(&self) -> Result<()>
    where
        J: Job,
    {
        lock_unpoisoned(&self.registries.job, "job registry").register::<J>()
    }

    pub fn register_job_middleware<M: JobMiddleware>(&self, middleware: M) -> Result<()> {
        lock_unpoisoned(&self.registries.job_middleware, "job middleware registry")
            .register(Arc::new(middleware));
        Ok(())
    }

    pub(crate) fn register_generated_migration_file<M>(
        &self,
        id: impl Into<MigrationId>,
    ) -> Result<()>
    where
        M: MigrationFile,
    {
        lock_unpoisoned(&self.registries.migration, "migration registry")
            .register_file::<M>(id.into())
    }

    pub(crate) fn register_generated_seeder_file<S>(&self, id: impl Into<SeederId>) -> Result<()>
    where
        S: SeederFile,
    {
        lock_unpoisoned(&self.registries.seeder, "seeder registry").register_file::<S>(id.into())
    }

    pub fn register_guard<I, G>(&self, id: I, guard: G) -> Result<()>
    where
        I: Into<GuardId>,
        G: BearerAuthenticator,
    {
        lock_unpoisoned(&self.registries.guard, "guard registry").register_arc(id, Arc::new(guard))
    }

    pub fn register_actor_hydrator<I, H>(&self, guard: I, hydrator: H) -> Result<()>
    where
        I: Into<GuardId>,
        H: ActorHydrator,
    {
        lock_unpoisoned(&self.registries.actor_hydrator, "actor hydrator registry")
            .register_arc(guard, Arc::new(hydrator))
    }

    pub fn register_policy<I, P>(&self, id: I, policy: P) -> Result<()>
    where
        I: Into<PolicyId>,
        P: Policy,
    {
        lock_unpoisoned(&self.registries.policy, "policy registry")
            .register_arc(id, Arc::new(policy))
    }

    pub fn register_authenticatable<M>(&self) -> Result<()>
    where
        M: Authenticatable,
    {
        lock_unpoisoned(&self.registries.authenticatable, "authenticatable registry")
            .register::<M>()
    }

    pub fn register_readiness_check<I, C>(&self, id: I, check: C) -> Result<()>
    where
        I: Into<ProbeId>,
        C: ReadinessCheck,
    {
        lock_unpoisoned(&self.registries.readiness, "readiness registry")
            .register_arc(id, Arc::new(check))
    }

    pub fn register_storage_driver(&self, name: &str, factory: StorageDriverFactory) -> Result<()> {
        lock_unpoisoned(&self.registries.storage_driver, "storage driver registry")
            .register(name.to_string(), factory)
    }

    pub fn register_email_driver(&self, name: &str, factory: EmailDriverFactory) -> Result<()> {
        lock_unpoisoned(&self.registries.email_driver, "email driver registry")
            .register(name.to_string(), factory)
    }

    pub fn register_notification_channel<I, N>(&self, id: I, channel: N) -> Result<()>
    where
        I: Into<crate::support::NotificationChannelId>,
        N: NotificationChannel,
    {
        lock_unpoisoned(
            &self.registries.notification_channel,
            "notification channel registry",
        )
        .register(id, Arc::new(channel))
    }

    pub(crate) fn notification_channel_registry(&self) -> NotificationChannelRegistryHandle {
        self.registries.notification_channel.clone()
    }

    pub(crate) fn job_middleware_registry(&self) -> JobMiddlewareRegistryHandle {
        self.registries.job_middleware.clone()
    }

    pub fn register_datatable<D>(&self) -> Result<()>
    where
        D: crate::datatable::Datatable,
    {
        lock_unpoisoned(&self.registries.datatable, "datatable registry").register::<D>()
    }

    pub(crate) fn datatable_registry(&self) -> DatatableRegistryHandle {
        self.registries.datatable.clone()
    }
}

#[async_trait]
pub trait ServiceProvider: Send + Sync + 'static {
    async fn register(&self, _registrar: &mut ServiceRegistrar) -> Result<()> {
        Ok(())
    }

    async fn boot(&self, _app: &crate::foundation::AppContext) -> Result<()> {
        Ok(())
    }
}
