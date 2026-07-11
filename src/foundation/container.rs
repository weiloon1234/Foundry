use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::foundation::{Error, Result};
use crate::logging::{catch_sync_panic, panic_payload_message};
use crate::support::sync::{read_unpoisoned, write_unpoisoned};

type SharedService = Arc<dyn Any + Send + Sync>;
type ServiceFactory = Arc<dyn Fn(&Container) -> Result<SharedService> + Send + Sync>;

#[derive(Clone)]
enum ServiceEntry {
    Singleton(SharedService),
    Factory(ServiceFactory),
}

#[derive(Clone, Default)]
pub struct Container {
    entries: Arc<RwLock<HashMap<TypeId, ServiceEntry>>>,
}

impl Container {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn singleton<T>(&self, value: T) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        self.singleton_arc(Arc::new(value))
    }

    pub fn singleton_arc<T>(&self, value: Arc<T>) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        let mut entries = write_unpoisoned(&self.entries, "container");
        let type_id = TypeId::of::<T>();
        if entries.contains_key(&type_id) {
            return Err(Error::message(format!(
                "service `{}` already registered",
                std::any::type_name::<T>()
            )));
        }

        let shared: SharedService = value;
        entries.insert(type_id, ServiceEntry::Singleton(shared));
        Ok(())
    }

    pub fn factory<T, F>(&self, factory: F) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn(&Container) -> Result<T> + Send + Sync + 'static,
    {
        self.factory_arc(move |container| {
            let value = factory(container)?;
            Ok(Arc::new(value))
        })
    }

    pub fn factory_arc<T, F>(&self, factory: F) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn(&Container) -> Result<Arc<T>> + Send + Sync + 'static,
    {
        let mut entries = write_unpoisoned(&self.entries, "container");
        let type_id = TypeId::of::<T>();
        if entries.contains_key(&type_id) {
            return Err(Error::message(format!(
                "service `{}` already registered",
                std::any::type_name::<T>()
            )));
        }

        let wrapped: ServiceFactory = Arc::new(move |container| {
            let service: Arc<T> = factory(container)?;
            let shared: SharedService = service;
            Ok(shared)
        });
        entries.insert(type_id, ServiceEntry::Factory(wrapped));
        Ok(())
    }

    pub fn resolve<T>(&self) -> Result<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        let entry = {
            let entries = read_unpoisoned(&self.entries, "container");
            entries.get(&TypeId::of::<T>()).cloned()
        }
        .ok_or_else(|| {
            Error::message(format!(
                "service `{}` not registered",
                std::any::type_name::<T>()
            ))
        })?;

        let shared = match entry {
            ServiceEntry::Singleton(value) => value,
            ServiceEntry::Factory(factory) => resolve_factory::<T>(&factory, self)?,
        };

        Arc::downcast::<T>(shared).map_err(|_| {
            Error::message(format!(
                "service `{}` registered with mismatched type",
                std::any::type_name::<T>()
            ))
        })
    }

    pub fn contains<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.entries
            .read()
            .ok()
            .and_then(|entries| entries.get(&TypeId::of::<T>()).cloned())
            .is_some()
    }

    pub(crate) fn replace_singleton_arc<T>(&self, value: Arc<T>) -> Result<()>
    where
        T: Send + Sync + 'static,
    {
        let mut entries = write_unpoisoned(&self.entries, "container");
        let type_id = TypeId::of::<T>();
        if !entries.contains_key(&type_id) {
            return Err(Error::message(format!(
                "test service `{}` is not registered and cannot be replaced",
                std::any::type_name::<T>()
            )));
        }

        let shared: SharedService = value;
        entries.insert(type_id, ServiceEntry::Singleton(shared));
        Ok(())
    }
}

fn resolve_factory<T>(factory: &ServiceFactory, container: &Container) -> Result<SharedService>
where
    T: Send + Sync + 'static,
{
    match catch_sync_panic(|| factory(container)) {
        Ok(result) => result,
        Err(panic) => Err(service_factory_panic_error(
            std::any::type_name::<T>(),
            panic,
        )),
    }
}

fn service_factory_panic_error(service: &'static str, panic: Box<dyn Any + Send>) -> Error {
    let message = panic_payload_message(panic);
    tracing::error!(
        service = service,
        panic = %message,
        "service factory panicked"
    );
    Error::message(format!("service factory `{service}` panicked: {message}"))
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::Arc;

    use super::Container;
    use crate::foundation::Error;

    #[test]
    fn resolves_singletons_and_factories() {
        let container = Container::new();
        container
            .singleton::<String>("foundry".to_string())
            .unwrap();
        container
            .factory::<usize, _>(|inner| Ok(inner.resolve::<String>()?.len()))
            .unwrap();

        assert_eq!(container.resolve::<String>().unwrap().as_str(), "foundry");
        assert_eq!(*container.resolve::<usize>().unwrap(), 7);
    }

    #[test]
    fn rejects_duplicate_registrations() {
        let container = Container::new();
        container
            .singleton::<String>("foundry".to_string())
            .unwrap();

        let error = container
            .singleton::<String>("duplicate".to_string())
            .unwrap_err();
        assert!(error.to_string().contains("already registered"));
    }

    #[test]
    fn test_replacement_requires_an_existing_registration() {
        let container = Container::new();
        let error = container
            .replace_singleton_arc(Arc::new(String::from("replacement")))
            .unwrap_err();
        assert!(error.to_string().contains("is not registered"));

        container.singleton(String::from("original")).unwrap();
        container
            .replace_singleton_arc(Arc::new(String::from("replacement")))
            .unwrap();
        assert_eq!(
            container.resolve::<String>().unwrap().as_str(),
            "replacement"
        );
    }

    #[test]
    fn factory_panic_becomes_error() {
        let container = Container::new();
        container
            .factory::<usize, _>(|_| panic!("factory exploded"))
            .unwrap();

        let error = container.resolve::<usize>().unwrap_err();
        let message = error.to_string();

        assert!(
            message.contains(&format!(
                "service factory `{}` panicked: factory exploded",
                std::any::type_name::<usize>()
            )),
            "{error}"
        );
    }

    #[test]
    fn factory_arc_panic_becomes_error() {
        let container = Container::new();
        container
            .factory_arc::<String, _>(|_| -> crate::foundation::Result<Arc<String>> {
                panic!("arc factory exploded")
            })
            .unwrap();

        let error = container.resolve::<String>().unwrap_err();
        let message = error.to_string();

        assert!(message.contains(&format!(
            "service factory `{}`",
            std::any::type_name::<String>()
        )));
        assert!(message.contains("panicked: arc factory exploded"));
    }

    #[test]
    fn factory_error_remains_unchanged() {
        let container = Container::new();
        container
            .factory::<usize, _>(|_| Err(Error::message("factory returned error")))
            .unwrap();

        let error = container.resolve::<usize>().unwrap_err();

        assert_eq!(error.to_string(), "factory returned error");
    }

    #[test]
    fn factory_panic_does_not_poison_other_resolution() {
        let container = Container::new();
        container
            .singleton::<String>("foundry".to_string())
            .unwrap();
        container
            .factory::<usize, _>(|_| panic!("factory exploded"))
            .unwrap();

        let message = container.resolve::<usize>().unwrap_err().to_string();

        assert!(message.contains(&format!(
            "service factory `{}` panicked: factory exploded",
            std::any::type_name::<usize>()
        )));
        assert_eq!(container.resolve::<String>().unwrap().as_str(), "foundry");
    }

    #[test]
    fn recovers_poisoned_entry_lock() {
        let container = Container::new();

        let result = catch_unwind(AssertUnwindSafe(|| {
            let _entries = container.entries.write().unwrap();
            panic!("poison container");
        }));
        assert!(result.is_err());

        container
            .singleton::<String>("foundry".to_string())
            .unwrap();

        assert_eq!(container.resolve::<String>().unwrap().as_str(), "foundry");
    }
}
