use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};

use ::redis::AsyncCommands;
use futures_util::StreamExt;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};
use tokio::task::JoinHandle;

use crate::config::ConfigRepository;
use crate::foundation::{Error, Result};
use crate::logging::RuntimeBackendKind;
use crate::redis::namespaced_value;
use crate::support::sync::lock_unpoisoned;
use crate::support::QueueId;

#[derive(Clone, Debug)]
pub(crate) struct PubSubMessage {
    pub topic: String,
    pub payload: String,
}

pub(crate) struct BackendSubscription {
    receiver: mpsc::UnboundedReceiver<PubSubMessage>,
    cancel: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl BackendSubscription {
    fn new(receiver: mpsc::UnboundedReceiver<PubSubMessage>) -> Self {
        Self {
            receiver,
            cancel: None,
            handle: None,
        }
    }

    fn with_task(
        receiver: mpsc::UnboundedReceiver<PubSubMessage>,
        cancel: oneshot::Sender<()>,
        handle: JoinHandle<()>,
    ) -> Self {
        Self {
            receiver,
            cancel: Some(cancel),
            handle: Some(handle),
        }
    }

    pub async fn recv(&mut self) -> Option<PubSubMessage> {
        self.receiver.recv().await
    }
}

impl Drop for BackendSubscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        let _ = self.handle.take();
    }
}

#[derive(Clone)]
pub(crate) enum RuntimeBackend {
    Redis(RedisRuntime),
    Memory(Arc<MemoryRuntime>),
}

impl RuntimeBackend {
    #[cfg(test)]
    pub fn memory(namespace: &str) -> Self {
        Self::Memory(shared_memory_runtime(namespace))
    }

    pub fn from_config(config: &ConfigRepository) -> Result<Self> {
        let redis = config.redis()?;
        let force_memory = std::env::var("FOUNDRY_INTERNAL_RUNTIME_BACKEND")
            .ok()
            .as_deref()
            == Some("memory");

        if force_memory || redis.url.trim().is_empty() {
            return Ok(Self::Memory(shared_memory_runtime(&redis.namespace)));
        }

        Ok(Self::Redis(RedisRuntime {
            client: ::redis::Client::open(redis.url.as_str()).map_err(Error::other)?,
            namespace: redis.namespace,
        }))
    }

    pub fn kind(&self) -> RuntimeBackendKind {
        match self {
            Self::Redis(_) => RuntimeBackendKind::Redis,
            Self::Memory(_) => RuntimeBackendKind::Memory,
        }
    }

    pub async fn ping(&self) -> Result<()> {
        match self {
            Self::Redis(runtime) => runtime.ping().await,
            Self::Memory(_) => Ok(()),
        }
    }

    pub async fn publish_ws(&self, topic: &str, payload: &str) -> Result<()> {
        match self {
            Self::Redis(runtime) => runtime.publish_ws(topic, payload).await,
            Self::Memory(runtime) => runtime.publish_ws(topic, payload).await,
        }
    }

    pub(crate) async fn subscribe_ws(&self, topics: &[String]) -> Result<BackendSubscription> {
        match self {
            Self::Redis(runtime) => runtime.subscribe_ws(topics).await,
            Self::Memory(runtime) => runtime.subscribe_ws(topics).await,
        }
    }

    /// Atomically increment a counter and set TTL on first creation.
    ///
    /// The key is automatically prefixed with the app's Redis namespace:
    /// `{namespace}:{key}`.
    ///
    /// Returns the current count after increment. If the key didn't exist
    /// before this call, it is created with value `1` and the given TTL.
    pub async fn incr_with_ttl(&self, key: &str, ttl_secs: u64) -> Result<u64> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                // Atomic INCR + conditional EXPIRE via Lua to prevent
                // leaked keys if the process crashes between the two commands
                let count: i64 = ::redis::cmd("EVAL")
                    .arg(
                        "local c = redis.call('INCR', KEYS[1]); \
                         if c == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end; \
                         return c",
                    )
                    .arg(1)
                    .arg(&full_key)
                    .arg(ttl_secs as i64)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(count as u64)
            }
            Self::Memory(runtime) => {
                let now = std::time::Instant::now();
                let ttl = std::time::Duration::from_secs(ttl_secs);
                let mut counters = runtime.counters.lock().await;
                let entry = counters.entry(key.to_string()).or_insert(MemoryCounter {
                    count: 0,
                    expires_at: now + ttl,
                });
                // Reset if expired
                if now >= entry.expires_at {
                    entry.count = 0;
                    entry.expires_at = now + ttl;
                }
                entry.count += 1;
                Ok(entry.count)
            }
        }
    }

    /// Add a member to a set.
    pub async fn sadd(&self, key: &str, member: &str) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let _: () = conn.sadd(full_key, member).await.map_err(Error::other)?;
                Ok(())
            }
            Self::Memory(runtime) => {
                let mut sets = runtime.sets.lock().await;
                sets.entry(key.to_string())
                    .or_default()
                    .insert(member.to_string());
                Ok(())
            }
        }
    }

    /// Remove a member from a set.
    pub async fn srem(&self, key: &str, member: &str) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let _: () = conn.srem(full_key, member).await.map_err(Error::other)?;
                Ok(())
            }
            Self::Memory(runtime) => {
                let mut sets = runtime.sets.lock().await;
                if let Some(set) = sets.get_mut(key) {
                    set.remove(member);
                    if set.is_empty() {
                        sets.remove(key);
                    }
                }
                Ok(())
            }
        }
    }

    /// Return all members of a set.
    pub async fn smembers(&self, key: &str) -> Result<Vec<String>> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let members: Vec<String> = conn.smembers(full_key).await.map_err(Error::other)?;
                Ok(members)
            }
            Self::Memory(runtime) => {
                let sets = runtime.sets.lock().await;
                Ok(sets
                    .get(key)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default())
            }
        }
    }

    /// Return the number of members in a set.
    pub async fn scard(&self, key: &str) -> Result<usize> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let count: usize = conn.scard(full_key).await.map_err(Error::other)?;
                Ok(count)
            }
            Self::Memory(runtime) => {
                let sets = runtime.sets.lock().await;
                Ok(sets.get(key).map(|s| s.len()).unwrap_or(0))
            }
        }
    }

    /// Check whether a key exists.
    ///
    /// Returns `true` if the key exists and has not expired.
    #[allow(dead_code)]
    pub async fn key_exists(&self, key: &str) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let exists: bool = ::redis::cmd("EXISTS")
                    .arg(&full_key)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(exists)
            }
            Self::Memory(runtime) => {
                let unique_keys = runtime.unique_keys.lock().await;
                if let Some((_, expires_at)) = unique_keys.get(key) {
                    Ok(std::time::Instant::now() < *expires_at)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// Unconditionally delete a key.
    ///
    /// Returns `true` if the key existed and was removed.
    pub async fn del_key(&self, key: &str) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let deleted: i64 = ::redis::cmd("DEL")
                    .arg(&full_key)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(deleted > 0)
            }
            Self::Memory(runtime) => {
                let mut unique_keys = runtime.unique_keys.lock().await;
                Ok(unique_keys.remove(key).is_some())
            }
        }
    }

    /// Get the string value of a key. Returns `None` if the key does not exist
    /// or has expired.
    pub async fn get_value(&self, key: &str) -> Result<Option<String>> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let value: Option<String> = ::redis::cmd("GET")
                    .arg(&full_key)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(value)
            }
            Self::Memory(runtime) => {
                let unique_keys = runtime.unique_keys.lock().await;
                if let Some((value, expires_at)) = unique_keys.get(key) {
                    if std::time::Instant::now() < *expires_at {
                        Ok(Some(value.clone()))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Set a key only if it does not already exist, with a TTL and a custom value.
    ///
    /// Returns `true` if the key was set (did not exist), `false` if
    /// it already existed.
    pub async fn set_nx_value(&self, key: &str, value: &str, ttl_secs: u64) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let result: Option<String> = ::redis::cmd("SET")
                    .arg(&full_key)
                    .arg(value)
                    .arg("NX")
                    .arg("EX")
                    .arg(ttl_secs as i64)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(result.is_some())
            }
            Self::Memory(runtime) => {
                let now = std::time::Instant::now();
                let ttl = std::time::Duration::from_secs(ttl_secs);
                let mut unique_keys = runtime.unique_keys.lock().await;
                // Evict expired entry
                if let Some((_, expires_at)) = unique_keys.get(key) {
                    if now >= *expires_at {
                        unique_keys.remove(key);
                    }
                }
                if unique_keys.contains_key(key) {
                    Ok(false)
                } else {
                    unique_keys.insert(key.to_string(), (value.to_string(), now + ttl));
                    Ok(true)
                }
            }
        }
    }

    /// Set a key to a value with a TTL, replacing any previous value.
    pub async fn set_value(&self, key: &str, value: &str, ttl_secs: u64) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let _: () = ::redis::cmd("SET")
                    .arg(&full_key)
                    .arg(value)
                    .arg("EX")
                    .arg(ttl_secs as i64)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(())
            }
            Self::Memory(runtime) => {
                let now = std::time::Instant::now();
                let ttl = std::time::Duration::from_secs(ttl_secs);
                let mut unique_keys = runtime.unique_keys.lock().await;
                unique_keys.insert(key.to_string(), (value.to_string(), now + ttl));
                Ok(())
            }
        }
    }

    /// Delete a key only if its current value matches the expected value.
    ///
    /// Returns `true` if the key was deleted.
    pub async fn del_if_value(&self, key: &str, expected: &str) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let deleted: i64 = ::redis::cmd("EVAL")
                    .arg(
                        r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('DEL', KEYS[1])
end
return 0
"#,
                    )
                    .arg(1)
                    .arg(&full_key)
                    .arg(expected)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(deleted == 1)
            }
            Self::Memory(runtime) => {
                let mut unique_keys = runtime.unique_keys.lock().await;
                if let Some((_, expires_at)) = unique_keys.get(key) {
                    if std::time::Instant::now() >= *expires_at {
                        unique_keys.remove(key);
                        return Ok(false);
                    }
                }
                if let Some((val, _)) = unique_keys.get(key) {
                    if val == expected {
                        unique_keys.remove(key);
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Extend a key expiration only if its current value matches the expected value.
    ///
    /// Returns `true` if the key was still owned by `expected` and its TTL was extended.
    pub async fn expire_if_value(&self, key: &str, expected: &str, ttl_secs: u64) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let extended: i64 = ::redis::cmd("EVAL")
                    .arg(
                        r#"
if redis.call('GET', KEYS[1]) == ARGV[1] then
    return redis.call('EXPIRE', KEYS[1], ARGV[2])
end
return 0
"#,
                    )
                    .arg(1)
                    .arg(&full_key)
                    .arg(expected)
                    .arg(ttl_secs as i64)
                    .query_async(&mut conn)
                    .await
                    .map_err(Error::other)?;
                Ok(extended == 1)
            }
            Self::Memory(runtime) => {
                let now = std::time::Instant::now();
                let ttl = std::time::Duration::from_secs(ttl_secs);
                let mut unique_keys = runtime.unique_keys.lock().await;
                if let Some((_, expires_at)) = unique_keys.get(key) {
                    if now >= *expires_at {
                        unique_keys.remove(key);
                        return Ok(false);
                    }
                }
                if let Some((value, expires_at)) = unique_keys.get_mut(key) {
                    if value == expected {
                        *expires_at = now + ttl;
                        return Ok(true);
                    }
                }
                Ok(false)
            }
        }
    }

    /// Push a value to the head of a list and trim to a maximum length (circular buffer).
    ///
    /// Equivalent to `LPUSH key value` followed by `LTRIM key 0 (max_len - 1)`.
    pub async fn lpush_capped(&self, key: &str, value: &str, max_len: usize) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let _: () = conn.lpush(&full_key, value).await.map_err(Error::other)?;
                let _: () = conn
                    .ltrim(&full_key, 0, max_len as isize - 1)
                    .await
                    .map_err(Error::other)?;
                Ok(())
            }
            Self::Memory(runtime) => {
                let mut lists = runtime.lists.lock().await;
                let list = lists.entry(key.to_string()).or_default();
                list.push_front(value.to_string());
                while list.len() > max_len {
                    list.pop_back();
                }
                Ok(())
            }
        }
    }

    /// Set an expiration on a key in seconds (`EXPIRE key ttl`).
    ///
    /// For the memory backend this only records the requested TTL — keys are
    /// not actually evicted — so tests can assert the correct TTL was applied
    /// without simulating wall-clock expiry.
    pub async fn expire(&self, key: &str, ttl_seconds: u64) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let _: bool = conn
                    .expire(&full_key, ttl_seconds as i64)
                    .await
                    .map_err(Error::other)?;
                Ok(())
            }
            Self::Memory(runtime) => {
                let mut ttls = runtime.ttls.lock().await;
                ttls.insert(key.to_string(), ttl_seconds);
                Ok(())
            }
        }
    }

    /// Return the remaining TTL for a key in seconds, or `None` if the key has
    /// no expiration / does not exist (`TTL key`).
    pub async fn ttl(&self, key: &str) -> Result<Option<u64>> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let seconds: i64 = conn.ttl(&full_key).await.map_err(Error::other)?;
                if seconds < 0 {
                    Ok(None)
                } else {
                    Ok(Some(seconds as u64))
                }
            }
            Self::Memory(runtime) => {
                let ttls = runtime.ttls.lock().await;
                Ok(ttls.get(key).copied())
            }
        }
    }

    /// Return a range of elements from a list.
    ///
    /// Equivalent to `LRANGE key start stop`.
    pub async fn lrange(&self, key: &str, start: i64, stop: i64) -> Result<Vec<String>> {
        match self {
            Self::Redis(runtime) => {
                let full_key = runtime.namespaced_key(key);
                let mut conn = runtime
                    .client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(Error::other)?;
                let values: Vec<String> = conn
                    .lrange(&full_key, start as isize, stop as isize)
                    .await
                    .map_err(Error::other)?;
                Ok(values)
            }
            Self::Memory(runtime) => {
                let lists = runtime.lists.lock().await;
                let Some(list) = lists.get(key) else {
                    return Ok(Vec::new());
                };
                let len = list.len() as i64;
                // Normalize negative indices like Redis does.
                let s = if start < 0 {
                    (len + start).max(0) as usize
                } else {
                    start as usize
                };
                let e = if stop < 0 {
                    (len + stop).max(0) as usize
                } else {
                    stop as usize
                };
                if s > e || s >= list.len() {
                    return Ok(Vec::new());
                }
                let end = e.min(list.len() - 1);
                Ok(list.iter().skip(s).take(end - s + 1).cloned().collect())
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct RedisRuntime {
    pub(crate) client: ::redis::Client,
    pub(crate) namespace: String,
}

impl RedisRuntime {
    fn websocket_topic(&self, topic: &str) -> String {
        namespaced_value(&self.namespace, &format!("ws:{topic}"))
    }

    /// Build a namespaced key: `{namespace}:{suffix}`.
    fn namespaced_key(&self, suffix: &str) -> String {
        namespaced_value(&self.namespace, suffix)
    }

    async fn ping(&self) -> Result<()> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(Error::other)?;
        let _: String = ::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(Error::other)?;
        Ok(())
    }

    async fn publish_ws(&self, topic: &str, payload: &str) -> Result<()> {
        let mut conn = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(Error::other)?;
        let redis_topic = self.websocket_topic(topic);
        let _: () = conn
            .publish(redis_topic, payload)
            .await
            .map_err(Error::other)?;
        Ok(())
    }

    async fn subscribe_ws(&self, topics: &[String]) -> Result<BackendSubscription> {
        let (tx, rx) = mpsc::unbounded_channel();
        if topics.is_empty() {
            return Ok(BackendSubscription::new(rx));
        }

        let mut pubsub = self.client.get_async_pubsub().await.map_err(Error::other)?;
        let mut logical_topics = HashMap::new();
        for topic in topics {
            let redis_topic = self.websocket_topic(topic);
            pubsub.subscribe(&redis_topic).await.map_err(Error::other)?;
            logical_topics.insert(redis_topic, topic.clone());
        }

        let mut stream = pubsub.into_on_message();
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            loop {
                let message = tokio::select! {
                    _ = &mut cancel_rx => break,
                    message = stream.next() => message,
                };
                let Some(message) = message else {
                    break;
                };
                let Ok(payload) = message.get_payload::<String>() else {
                    continue;
                };
                let raw_topic = message.get_channel_name().to_string();
                let topic = logical_topics.get(&raw_topic).cloned().unwrap_or(raw_topic);
                if tx.send(PubSubMessage { topic, payload }).is_err() {
                    break;
                }
            }
        });

        Ok(BackendSubscription::with_task(rx, cancel_tx, handle))
    }
}

/// Batch metadata stored in the memory backend.
#[derive(Clone, Debug)]
pub(crate) struct MemoryBatchMeta {
    pub total: u64,
    pub completed: u64,
    pub on_complete_job: Option<String>,
    pub on_complete_queue: Option<String>,
    pub on_complete_dispatched: bool,
}

pub(crate) struct MemoryRuntime {
    pub(crate) ws_tx: broadcast::Sender<PubSubMessage>,
    pub(crate) ready_queues: Mutex<HashMap<QueueId, VecDeque<String>>>,
    pub(crate) scheduled_jobs: Mutex<HashMap<QueueId, Vec<ScheduledJobToken>>>,
    pub(crate) leased_jobs: Mutex<HashMap<QueueId, Vec<LeasedJobToken>>>,
    pub(crate) payloads: Mutex<HashMap<String, String>>,
    pub(crate) dead_letters: Mutex<HashMap<QueueId, Vec<String>>>,
    pub(crate) scheduler_leader: Mutex<Option<LeadershipLease>>,
    pub(crate) batches: Mutex<HashMap<String, MemoryBatchMeta>>,
    pub(crate) counters: Mutex<HashMap<String, MemoryCounter>>,
    pub(crate) unique_keys: Mutex<HashMap<String, (String, std::time::Instant)>>,
    pub(crate) sets: Mutex<HashMap<String, HashSet<String>>>,
    pub(crate) lists: Mutex<HashMap<String, VecDeque<String>>>,
    pub(crate) ttls: Mutex<HashMap<String, u64>>,
    pub(crate) notify: Notify,
}

/// In-memory counter with TTL-based expiration.
pub(crate) struct MemoryCounter {
    pub count: u64,
    pub expires_at: std::time::Instant,
}

#[derive(Clone)]
pub(crate) struct ScheduledJobToken {
    pub run_at_millis: i64,
    pub token: String,
}

#[derive(Clone)]
pub(crate) struct LeasedJobToken {
    pub expires_at_millis: i64,
    pub token: String,
}

#[derive(Clone)]
pub(crate) struct LeadershipLease {
    pub owner_id: String,
    pub expires_at_millis: i64,
}

impl MemoryRuntime {
    fn new() -> Self {
        let (ws_tx, _) = broadcast::channel(1024);
        Self {
            ws_tx,
            ready_queues: Mutex::new(HashMap::new()),
            scheduled_jobs: Mutex::new(HashMap::new()),
            leased_jobs: Mutex::new(HashMap::new()),
            payloads: Mutex::new(HashMap::new()),
            dead_letters: Mutex::new(HashMap::new()),
            scheduler_leader: Mutex::new(None),
            batches: Mutex::new(HashMap::new()),
            counters: Mutex::new(HashMap::new()),
            unique_keys: Mutex::new(HashMap::new()),
            sets: Mutex::new(HashMap::new()),
            lists: Mutex::new(HashMap::new()),
            ttls: Mutex::new(HashMap::new()),
            notify: Notify::new(),
        }
    }

    async fn publish_ws(&self, topic: &str, payload: &str) -> Result<()> {
        let _ = self.ws_tx.send(PubSubMessage {
            topic: topic.to_string(),
            payload: payload.to_string(),
        });
        Ok(())
    }

    async fn subscribe_ws(&self, topics: &[String]) -> Result<BackendSubscription> {
        let topics = topics.to_vec();
        let mut receiver = self.ws_tx.subscribe();
        let (tx, rx) = mpsc::unbounded_channel();
        if topics.is_empty() {
            return Ok(BackendSubscription::new(rx));
        }

        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            loop {
                let message = tokio::select! {
                    _ = &mut cancel_rx => break,
                    message = receiver.recv() => message,
                };
                match message {
                    Ok(message) => {
                        if topics.iter().any(|topic| topic == &message.topic)
                            && tx.send(message).is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(BackendSubscription::with_task(rx, cancel_tx, handle))
    }
}

fn shared_memory_runtime(namespace: &str) -> Arc<MemoryRuntime> {
    static REGISTRY: OnceLock<StdMutex<HashMap<String, Weak<MemoryRuntime>>>> = OnceLock::new();

    let registry = REGISTRY.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut registry = lock_unpoisoned(registry, "runtime registry");

    if let Some(existing) = registry.get(namespace).and_then(Weak::upgrade) {
        return existing;
    }

    let runtime = Arc::new(MemoryRuntime::new());
    registry.insert(namespace.to_string(), Arc::downgrade(&runtime));
    runtime
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::RuntimeBackend;

    fn memory_backend() -> RuntimeBackend {
        RuntimeBackend::memory(&format!("test-{}", Uuid::now_v7()))
    }

    #[tokio::test]
    async fn memory_subscription_receives_matching_topics() {
        let backend = memory_backend();
        let mut subscription = backend.subscribe_ws(&["chat".to_string()]).await.unwrap();

        backend.publish_ws("other", "ignored").await.unwrap();
        backend.publish_ws("chat", "hello").await.unwrap();

        let message = subscription.recv().await.unwrap();
        assert_eq!(message.topic, "chat");
        assert_eq!(message.payload, "hello");
    }

    #[tokio::test]
    async fn dropping_memory_subscription_stops_forwarder() {
        let backend = memory_backend();
        let runtime = match &backend {
            RuntimeBackend::Memory(runtime) => runtime.clone(),
            RuntimeBackend::Redis(_) => unreachable!("test backend is memory"),
        };

        let subscription = backend.subscribe_ws(&["chat".to_string()]).await.unwrap();

        for _ in 0..20 {
            if runtime.ws_tx.receiver_count() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(runtime.ws_tx.receiver_count(), 1);

        drop(subscription);

        for _ in 0..20 {
            if runtime.ws_tx.receiver_count() == 0 {
                return;
            }
            tokio::task::yield_now().await;
        }

        assert_eq!(runtime.ws_tx.receiver_count(), 0);
    }

    #[tokio::test]
    async fn empty_subscription_does_not_spawn_forwarder() {
        let backend = memory_backend();
        let subscription = backend.subscribe_ws(&[]).await.unwrap();

        assert!(subscription.handle.is_none());
    }
}
