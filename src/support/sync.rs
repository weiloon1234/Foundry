use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(crate) fn lock_unpoisoned<'a, T: ?Sized>(
    mutex: &'a Mutex<T>,
    name: &'static str,
) -> MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn_poisoned(name);
            poisoned.into_inner()
        }
    }
}

pub(crate) fn mutex_into_inner_unpoisoned<T>(mutex: Mutex<T>, name: &'static str) -> T {
    match mutex.into_inner() {
        Ok(value) => value,
        Err(poisoned) => {
            warn_poisoned(name);
            poisoned.into_inner()
        }
    }
}

pub(crate) fn read_unpoisoned<'a, T: ?Sized>(
    lock: &'a RwLock<T>,
    name: &'static str,
) -> RwLockReadGuard<'a, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn_poisoned(name);
            poisoned.into_inner()
        }
    }
}

pub(crate) fn write_unpoisoned<'a, T: ?Sized>(
    lock: &'a RwLock<T>,
    name: &'static str,
) -> RwLockWriteGuard<'a, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn_poisoned(name);
            poisoned.into_inner()
        }
    }
}

fn warn_poisoned(name: &'static str) {
    tracing::warn!(
        target: "foundry.sync",
        lock = name,
        "recovering poisoned lock"
    );
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{Mutex, RwLock};

    use super::lock_unpoisoned;

    #[test]
    fn lock_unpoisoned_recovers_poisoned_mutex() {
        let lock = Mutex::new(Vec::new());

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut values = lock.lock().unwrap();
            values.push("before panic");
            panic!("poison lock");
        }));
        assert!(result.is_err());

        let mut values = lock_unpoisoned(&lock, "test lock");
        values.push("after recovery");

        assert_eq!(values.as_slice(), ["before panic", "after recovery"]);
    }

    #[test]
    fn mutex_into_inner_recovers_poisoned_mutex() {
        let lock = Mutex::new(Vec::new());

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut values = lock.lock().unwrap();
            values.push("before panic");
            panic!("poison lock");
        }));
        assert!(result.is_err());

        let values = super::mutex_into_inner_unpoisoned(lock, "test lock");

        assert_eq!(values.as_slice(), ["before panic"]);
    }

    #[test]
    fn rwlock_helpers_recover_poisoned_lock() {
        let lock = RwLock::new(Vec::new());

        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut values = lock.write().unwrap();
            values.push("before panic");
            panic!("poison lock");
        }));
        assert!(result.is_err());

        {
            let mut values = super::write_unpoisoned(&lock, "test rwlock");
            values.push("after recovery");
        }

        let values = super::read_unpoisoned(&lock, "test rwlock");
        assert_eq!(values.as_slice(), ["before panic", "after recovery"]);
    }
}
