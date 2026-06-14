use std::time::Duration;

use crate::foundation::{Error, Result};
use crate::redis::namespaced_value;
use crate::support::runtime::{MemoryRuntime, RedisRuntime, RuntimeBackend};

const RENEW_LEADERSHIP_SCRIPT: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
  redis.call('PSETEX', KEYS[1], ARGV[2], ARGV[1])
  return 1
end
return 0
"#;

const RELEASE_LEADERSHIP_SCRIPT: &str = r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
  redis.call('DEL', KEYS[1])
  return 1
end
return 0
"#;

impl RuntimeBackend {
    pub(crate) async fn try_acquire_scheduler_leadership(
        &self,
        owner_id: &str,
        ttl: Duration,
    ) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                acquire_scheduler_leadership_redis(runtime, owner_id, ttl).await
            }
            Self::Memory(runtime) => {
                acquire_scheduler_leadership_memory(runtime, owner_id, ttl).await
            }
        }
    }

    pub(crate) async fn renew_scheduler_leadership(
        &self,
        owner_id: &str,
        ttl: Duration,
    ) -> Result<bool> {
        match self {
            Self::Redis(runtime) => renew_scheduler_leadership_redis(runtime, owner_id, ttl).await,
            Self::Memory(runtime) => {
                renew_scheduler_leadership_memory(runtime, owner_id, ttl).await
            }
        }
    }

    pub(crate) async fn release_scheduler_leadership(&self, owner_id: &str) -> Result<()> {
        match self {
            Self::Redis(runtime) => release_scheduler_leadership_redis(runtime, owner_id).await,
            Self::Memory(runtime) => release_scheduler_leadership_memory(runtime, owner_id).await,
        }
    }
}

fn leadership_key(runtime: &RedisRuntime) -> String {
    namespaced_value(&runtime.namespace, "scheduler:leader")
}

fn ttl_millis(ttl: Duration) -> u64 {
    ttl.as_millis().max(1) as u64
}

fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

async fn acquire_scheduler_leadership_redis(
    runtime: &RedisRuntime,
    owner_id: &str,
    ttl: Duration,
) -> Result<bool> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let result: Option<String> = ::redis::cmd("SET")
        .arg(leadership_key(runtime))
        .arg(owner_id)
        .arg("NX")
        .arg("PX")
        .arg(ttl_millis(ttl))
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(result.is_some())
}

async fn renew_scheduler_leadership_redis(
    runtime: &RedisRuntime,
    owner_id: &str,
    ttl: Duration,
) -> Result<bool> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let renewed: i64 = ::redis::cmd("EVAL")
        .arg(RENEW_LEADERSHIP_SCRIPT)
        .arg(1)
        .arg(leadership_key(runtime))
        .arg(owner_id)
        .arg(ttl_millis(ttl))
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(renewed == 1)
}

async fn release_scheduler_leadership_redis(runtime: &RedisRuntime, owner_id: &str) -> Result<()> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let _: i64 = ::redis::cmd("EVAL")
        .arg(RELEASE_LEADERSHIP_SCRIPT)
        .arg(1)
        .arg(leadership_key(runtime))
        .arg(owner_id)
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(())
}

async fn acquire_scheduler_leadership_memory(
    runtime: &MemoryRuntime,
    owner_id: &str,
    ttl: Duration,
) -> Result<bool> {
    let mut leader = runtime.scheduler_leader.lock().await;
    let now = now_millis();
    match leader.as_ref() {
        Some(existing) if existing.expires_at_millis > now && existing.owner_id != owner_id => {
            Ok(false)
        }
        _ => {
            *leader = Some(crate::support::runtime::LeadershipLease {
                owner_id: owner_id.to_string(),
                expires_at_millis: now + ttl.as_millis() as i64,
            });
            Ok(true)
        }
    }
}

async fn renew_scheduler_leadership_memory(
    runtime: &MemoryRuntime,
    owner_id: &str,
    ttl: Duration,
) -> Result<bool> {
    let mut leader = runtime.scheduler_leader.lock().await;
    let now = now_millis();
    match leader.as_mut() {
        Some(existing) if existing.owner_id == owner_id && existing.expires_at_millis > now => {
            existing.expires_at_millis = now + ttl.as_millis() as i64;
            Ok(true)
        }
        Some(existing) if existing.owner_id == owner_id => {
            // Expired lease: mirror Redis, where the key is gone and renewal
            // fails. The previous holder must win a fresh election instead of
            // silently re-asserting leadership another instance may now hold.
            *leader = None;
            Ok(false)
        }
        _ => Ok(false),
    }
}

async fn release_scheduler_leadership_memory(
    runtime: &MemoryRuntime,
    owner_id: &str,
) -> Result<()> {
    let mut leader = runtime.scheduler_leader.lock().await;
    if leader
        .as_ref()
        .map(|existing| existing.owner_id == owner_id)
        .unwrap_or(false)
    {
        *leader = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::RuntimeBackend;

    #[tokio::test]
    async fn memory_backend_only_allows_one_scheduler_leader() {
        let backend = RuntimeBackend::memory("scheduler-leader-unit");

        assert!(backend
            .try_acquire_scheduler_leadership("leader-a", Duration::from_millis(50))
            .await
            .unwrap());
        assert!(!backend
            .try_acquire_scheduler_leadership("leader-b", Duration::from_millis(50))
            .await
            .unwrap());
        assert!(backend
            .renew_scheduler_leadership("leader-a", Duration::from_millis(50))
            .await
            .unwrap());
        backend
            .release_scheduler_leadership("leader-a")
            .await
            .unwrap();
        assert!(backend
            .try_acquire_scheduler_leadership("leader-b", Duration::from_millis(50))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn memory_backend_rejects_renewal_of_expired_lease() {
        let backend = RuntimeBackend::memory("scheduler-leader-expiry-unit");

        assert!(backend
            .try_acquire_scheduler_leadership("leader-a", Duration::from_millis(10))
            .await
            .unwrap());
        tokio::time::sleep(Duration::from_millis(30)).await;

        // The lease expired, so the old holder must not be able to renew it;
        // it has to win a fresh election like any other instance.
        assert!(!backend
            .renew_scheduler_leadership("leader-a", Duration::from_millis(50))
            .await
            .unwrap());
        assert!(backend
            .try_acquire_scheduler_leadership("leader-b", Duration::from_millis(50))
            .await
            .unwrap());
    }
}
