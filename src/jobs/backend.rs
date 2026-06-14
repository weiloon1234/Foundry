use std::time::Duration;

use crate::foundation::{Error, Result};
use crate::redis::namespaced_value;
use crate::support::runtime::{
    LeasedJobToken, MemoryRuntime, RedisRuntime, RuntimeBackend, ScheduledJobToken,
};
use crate::support::QueueId;

const CLAIM_STALE_TOKEN_SCAN_LIMIT: usize = 1024;

const CLAIM_JOB_SCRIPT: &str = r#"
local scanned = 0
local scan_limit = tonumber(ARGV[2])
while scanned < scan_limit do
  local token = redis.call('LPOP', KEYS[1])
  if not token then
    return nil
  end
  scanned = scanned + 1
  local payload = redis.call('HGET', KEYS[2], token)
  if payload and not redis.call('ZSCORE', KEYS[3], token) then
    redis.call('ZADD', KEYS[3], ARGV[1], token)
    return {token, payload}
  end
end
return nil
"#;

const PROMOTE_DUE_SCRIPT: &str = r#"
local tokens = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
if #tokens == 0 then
  return 0
end
redis.call('ZREM', KEYS[1], unpack(tokens))
local ready_tokens = {}
for _, token in ipairs(tokens) do
  if redis.call('HEXISTS', KEYS[3], token) == 1 then
    table.insert(ready_tokens, token)
  end
end
if #ready_tokens > 0 then
  redis.call('RPUSH', KEYS[2], unpack(ready_tokens))
end
return #ready_tokens
"#;

const REQUEUE_EXPIRED_SCRIPT: &str = r#"
local tokens = redis.call('ZRANGEBYSCORE', KEYS[1], '-inf', ARGV[1], 'LIMIT', 0, ARGV[2])
if #tokens == 0 then
  return 0
end
redis.call('ZREM', KEYS[1], unpack(tokens))
local ready_tokens = {}
for _, token in ipairs(tokens) do
  if redis.call('HEXISTS', KEYS[3], token) == 1 then
    table.insert(ready_tokens, token)
  end
end
if #ready_tokens > 0 then
  redis.call('RPUSH', KEYS[2], unpack(ready_tokens))
end
return #ready_tokens
"#;

const RENEW_LEASE_SCRIPT: &str = r#"
if redis.call('ZSCORE', KEYS[1], ARGV[1]) then
  redis.call('ZADD', KEYS[1], ARGV[2], ARGV[1])
  return 1
end
return 0
"#;

const RETRY_JOB_SCRIPT: &str = r#"
if redis.call('ZREM', KEYS[1], ARGV[1]) == 0 then
  return 0
end
redis.call('HDEL', KEYS[2], ARGV[1])
redis.call('HSET', KEYS[2], ARGV[2], ARGV[3])
redis.call('ZADD', KEYS[3], ARGV[4], ARGV[2])
return 1
"#;

const DEAD_LETTER_JOB_SCRIPT: &str = r#"
if redis.call('ZREM', KEYS[1], ARGV[1]) == 0 then
  return 0
end
redis.call('HDEL', KEYS[2], ARGV[1])
redis.call('RPUSH', KEYS[3], ARGV[2])
return 1
"#;

const COMPLETE_SUCCESSFUL_JOB_SCRIPT: &str = r#"
local function namespaced(suffix)
  if ARGV[1] == '' then
    return suffix
  end
  return ARGV[1] .. ':' .. suffix
end

if redis.call('ZREM', KEYS[1], ARGV[2]) == 0 then
  return {0, 0, 0, 0, 0, 0}
end
redis.call('HDEL', KEYS[2], ARGV[2])

local chain_enqueued = 0
if ARGV[3] ~= '' then
  local chain_queue = ARGV[5]
  redis.call('HSET', namespaced('jobs:payload:' .. chain_queue), ARGV[3], ARGV[4])
  redis.call('RPUSH', namespaced('jobs:ready:' .. chain_queue), ARGV[3])
  chain_enqueued = 1
end

local batch_found = 0
local batch_completed = 0
local batch_total = 0
local batch_callback_enqueued = 0
if ARGV[6] ~= '' then
  local batch_key = namespaced('jobs:batch:' .. ARGV[6])
  local total = redis.call('HGET', batch_key, 'total')
  if total then
    batch_found = 1
    batch_total = tonumber(total) or 0
    batch_completed = redis.call('HINCRBY', batch_key, 'completed', 1)

    local callback_payload = redis.call('HGET', batch_key, 'on_complete_payload') or ''
    local callback_dispatched = redis.call('HGET', batch_key, 'on_complete_dispatched') or '0'
    if batch_total > 0
      and batch_completed >= batch_total
      and callback_payload ~= ''
      and callback_dispatched ~= '1'
      and ARGV[8] ~= ''
    then
      local callback_queue = redis.call('HGET', batch_key, 'on_complete_queue') or ARGV[7]
      if callback_queue == '' then
        callback_queue = ARGV[7]
      end
      redis.call('HSET', namespaced('jobs:payload:' .. callback_queue), ARGV[8], callback_payload)
      redis.call('RPUSH', namespaced('jobs:ready:' .. callback_queue), ARGV[8])
      redis.call('HSET', batch_key, 'on_complete_dispatched', '1')
      batch_callback_enqueued = 1
    end
  end
end

return {1, chain_enqueued, batch_found, batch_completed, batch_total, batch_callback_enqueued}
"#;

const DISPATCH_BATCH_SCRIPT: &str = r#"
local function namespaced(suffix)
  if ARGV[1] == '' then
    return suffix
  end
  return ARGV[1] .. ':' .. suffix
end

local batch_id = ARGV[2]
local total = tonumber(ARGV[3]) or 0
local batch_key = namespaced('jobs:batch:' .. batch_id)
redis.call('HSET', batch_key, 'total', total, 'completed', '0', 'on_complete_dispatched', '0')
if ARGV[4] ~= '' then
  redis.call('HSET', batch_key, 'on_complete_payload', ARGV[4])
end
if ARGV[5] ~= '' then
  redis.call('HSET', batch_key, 'on_complete_queue', ARGV[5])
end
redis.call('EXPIRE', batch_key, 86400)

local count = tonumber(ARGV[6]) or 0
local index = 7
for _ = 1, count do
  local queue = ARGV[index]
  local token = ARGV[index + 1]
  local payload = ARGV[index + 2]
  redis.call('HSET', namespaced('jobs:payload:' .. queue), token, payload)
  redis.call('RPUSH', namespaced('jobs:ready:' .. queue), token)
  index = index + 3
end

return count
"#;

#[derive(Clone, Debug)]
pub(crate) struct ClaimedJobLease {
    pub queue: QueueId,
    pub token: String,
    pub payload: String,
}

#[derive(Clone, Debug)]
pub(crate) struct JobToEnqueue {
    pub queue: QueueId,
    pub token: String,
    pub payload: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SuccessfulJobEffects {
    pub chain: Option<JobToEnqueue>,
    pub batch_id: Option<String>,
    pub batch_callback_token: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct SuccessfulJobCompletion {
    pub lease_released: bool,
    pub chain_enqueued: bool,
    pub batch: Option<BatchCompletion>,
}

#[derive(Clone, Debug)]
pub(crate) struct BatchCompletion {
    pub completed: u64,
    pub total: u64,
    pub on_complete_enqueued: bool,
}

impl RuntimeBackend {
    pub(crate) async fn enqueue_job(
        &self,
        queue: &QueueId,
        token: &str,
        payload: &str,
    ) -> Result<()> {
        match self {
            Self::Redis(runtime) => enqueue_job_redis(runtime, queue, token, payload).await,
            Self::Memory(runtime) => enqueue_job_memory(runtime, queue, token, payload).await,
        }
    }

    pub(crate) async fn schedule_job(
        &self,
        queue: &QueueId,
        token: &str,
        payload: &str,
        run_at_millis: i64,
    ) -> Result<()> {
        match self {
            Self::Redis(runtime) => {
                schedule_job_redis(runtime, queue, token, payload, run_at_millis).await
            }
            Self::Memory(runtime) => {
                schedule_job_memory(runtime, queue, token, payload, run_at_millis).await
            }
        }
    }

    pub(crate) async fn promote_due_jobs(
        &self,
        queues: &[QueueId],
        now_millis: i64,
        limit: usize,
    ) -> Result<usize> {
        match self {
            Self::Redis(runtime) => {
                promote_due_jobs_redis(runtime, queues, now_millis, limit).await
            }
            Self::Memory(runtime) => {
                promote_due_jobs_memory(runtime, queues, now_millis, limit).await
            }
        }
    }

    pub(crate) async fn requeue_expired_jobs(
        &self,
        queues: &[QueueId],
        now_millis: i64,
        limit: usize,
    ) -> Result<usize> {
        match self {
            Self::Redis(runtime) => {
                requeue_expired_jobs_redis(runtime, queues, now_millis, limit).await
            }
            Self::Memory(runtime) => {
                requeue_expired_jobs_memory(runtime, queues, now_millis, limit).await
            }
        }
    }

    pub(crate) async fn claim_job(
        &self,
        queues: &[QueueId],
        lease_ttl: Duration,
    ) -> Result<Option<ClaimedJobLease>> {
        match self {
            Self::Redis(runtime) => claim_job_redis(runtime, queues, lease_ttl).await,
            Self::Memory(runtime) => claim_job_memory(runtime, queues, lease_ttl).await,
        }
    }

    pub(crate) async fn renew_job_lease(
        &self,
        queue: &QueueId,
        token: &str,
        lease_ttl: Duration,
    ) -> Result<bool> {
        match self {
            Self::Redis(runtime) => renew_job_lease_redis(runtime, queue, token, lease_ttl).await,
            Self::Memory(runtime) => renew_job_lease_memory(runtime, queue, token, lease_ttl).await,
        }
    }

    pub(crate) async fn retry_job(
        &self,
        queue: &QueueId,
        token: &str,
        new_token: &str,
        payload: &str,
        run_at_millis: i64,
    ) -> Result<bool> {
        match self {
            Self::Redis(runtime) => {
                retry_job_redis(runtime, queue, token, new_token, payload, run_at_millis).await
            }
            Self::Memory(runtime) => {
                retry_job_memory(runtime, queue, token, new_token, payload, run_at_millis).await
            }
        }
    }

    pub(crate) async fn dead_letter_job(
        &self,
        queue: &QueueId,
        token: &str,
        payload: &str,
    ) -> Result<bool> {
        match self {
            Self::Redis(runtime) => dead_letter_job_redis(runtime, queue, token, payload).await,
            Self::Memory(runtime) => dead_letter_job_memory(runtime, queue, token, payload).await,
        }
    }

    pub(crate) async fn complete_successful_job(
        &self,
        queue: &QueueId,
        token: &str,
        default_queue: &QueueId,
        effects: SuccessfulJobEffects,
    ) -> Result<SuccessfulJobCompletion> {
        match self {
            Self::Redis(runtime) => {
                complete_successful_job_redis(runtime, queue, token, default_queue, effects).await
            }
            Self::Memory(runtime) => {
                complete_successful_job_memory(runtime, queue, token, default_queue, effects).await
            }
        }
    }

    pub(crate) async fn dispatch_batch(
        &self,
        batch_id: &str,
        on_complete_payload: Option<&str>,
        on_complete_queue: Option<&str>,
        jobs: Vec<JobToEnqueue>,
    ) -> Result<usize> {
        match self {
            Self::Redis(runtime) => {
                dispatch_batch_redis(
                    runtime,
                    batch_id,
                    on_complete_payload,
                    on_complete_queue,
                    jobs,
                )
                .await
            }
            Self::Memory(runtime) => {
                dispatch_batch_memory(
                    runtime,
                    batch_id,
                    on_complete_payload,
                    on_complete_queue,
                    jobs,
                )
                .await
            }
        }
    }

    #[cfg(test)]
    pub(crate) async fn dead_letters(&self, queue: &QueueId) -> Result<Vec<String>> {
        match self {
            Self::Redis(_) => Ok(Vec::new()),
            Self::Memory(runtime) => dead_letters_memory(runtime, queue).await,
        }
    }
}

fn ready_key(runtime: &RedisRuntime, queue: &QueueId) -> String {
    namespaced_value(&runtime.namespace, &format!("jobs:ready:{queue}"))
}

fn scheduled_key(runtime: &RedisRuntime, queue: &QueueId) -> String {
    namespaced_value(&runtime.namespace, &format!("jobs:scheduled:{queue}"))
}

fn leased_key(runtime: &RedisRuntime, queue: &QueueId) -> String {
    namespaced_value(&runtime.namespace, &format!("jobs:leased:{queue}"))
}

fn payload_key(runtime: &RedisRuntime, queue: &QueueId) -> String {
    namespaced_value(&runtime.namespace, &format!("jobs:payload:{queue}"))
}

fn dead_letter_key(runtime: &RedisRuntime, queue: &QueueId) -> String {
    namespaced_value(&runtime.namespace, &format!("jobs:dead:{queue}"))
}

fn expires_at(lease_ttl: Duration) -> i64 {
    chrono::Utc::now().timestamp_millis() + lease_ttl.as_millis() as i64
}

async fn enqueue_job_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
) -> Result<()> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let _: () = ::redis::pipe()
        .atomic()
        .cmd("HSET")
        .arg(payload_key(runtime, queue))
        .arg(token)
        .arg(payload)
        .ignore()
        .cmd("RPUSH")
        .arg(ready_key(runtime, queue))
        .arg(token)
        .ignore()
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(())
}

async fn schedule_job_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
    run_at_millis: i64,
) -> Result<()> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let _: () = ::redis::pipe()
        .atomic()
        .cmd("HSET")
        .arg(payload_key(runtime, queue))
        .arg(token)
        .arg(payload)
        .ignore()
        .cmd("ZADD")
        .arg(scheduled_key(runtime, queue))
        .arg(run_at_millis)
        .arg(token)
        .ignore()
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(())
}

async fn promote_due_jobs_redis(
    runtime: &RedisRuntime,
    queues: &[QueueId],
    now_millis: i64,
    limit: usize,
) -> Result<usize> {
    let mut moved = 0usize;
    for queue in queues {
        if moved >= limit {
            break;
        }

        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(Error::other)?;
        let count: i64 = ::redis::cmd("EVAL")
            .arg(PROMOTE_DUE_SCRIPT)
            .arg(3)
            .arg(scheduled_key(runtime, queue))
            .arg(ready_key(runtime, queue))
            .arg(payload_key(runtime, queue))
            .arg(now_millis)
            .arg((limit - moved) as i64)
            .query_async(&mut conn)
            .await
            .map_err(Error::other)?;
        moved += count.max(0) as usize;
    }

    Ok(moved)
}

async fn requeue_expired_jobs_redis(
    runtime: &RedisRuntime,
    queues: &[QueueId],
    now_millis: i64,
    limit: usize,
) -> Result<usize> {
    let mut moved = 0usize;
    for queue in queues {
        if moved >= limit {
            break;
        }

        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(Error::other)?;
        let count: i64 = ::redis::cmd("EVAL")
            .arg(REQUEUE_EXPIRED_SCRIPT)
            .arg(3)
            .arg(leased_key(runtime, queue))
            .arg(ready_key(runtime, queue))
            .arg(payload_key(runtime, queue))
            .arg(now_millis)
            .arg((limit - moved) as i64)
            .query_async(&mut conn)
            .await
            .map_err(Error::other)?;
        moved += count.max(0) as usize;
    }

    Ok(moved)
}

async fn claim_job_redis(
    runtime: &RedisRuntime,
    queues: &[QueueId],
    lease_ttl: Duration,
) -> Result<Option<ClaimedJobLease>> {
    let lease_expires_at = expires_at(lease_ttl);
    for queue in queues {
        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(Error::other)?;
        let result: Option<Vec<String>> = ::redis::cmd("EVAL")
            .arg(CLAIM_JOB_SCRIPT)
            .arg(3)
            .arg(ready_key(runtime, queue))
            .arg(payload_key(runtime, queue))
            .arg(leased_key(runtime, queue))
            .arg(lease_expires_at)
            .arg(CLAIM_STALE_TOKEN_SCAN_LIMIT as i64)
            .query_async(&mut conn)
            .await
            .map_err(Error::other)?;

        if let Some(values) = result {
            if values.len() == 2 {
                return Ok(Some(ClaimedJobLease {
                    queue: queue.clone(),
                    token: values[0].clone(),
                    payload: values[1].clone(),
                }));
            }
        }
    }

    Ok(None)
}

async fn renew_job_lease_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    lease_ttl: Duration,
) -> Result<bool> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let renewed: i64 = ::redis::cmd("EVAL")
        .arg(RENEW_LEASE_SCRIPT)
        .arg(1)
        .arg(leased_key(runtime, queue))
        .arg(token)
        .arg(expires_at(lease_ttl))
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(renewed == 1)
}

async fn retry_job_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    new_token: &str,
    payload: &str,
    run_at_millis: i64,
) -> Result<bool> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let rescheduled: i64 = ::redis::cmd("EVAL")
        .arg(RETRY_JOB_SCRIPT)
        .arg(3)
        .arg(leased_key(runtime, queue))
        .arg(payload_key(runtime, queue))
        .arg(scheduled_key(runtime, queue))
        .arg(token)
        .arg(new_token)
        .arg(payload)
        .arg(run_at_millis)
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(rescheduled == 1)
}

async fn dead_letter_job_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
) -> Result<bool> {
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let dead_lettered: i64 = ::redis::cmd("EVAL")
        .arg(DEAD_LETTER_JOB_SCRIPT)
        .arg(3)
        .arg(leased_key(runtime, queue))
        .arg(payload_key(runtime, queue))
        .arg(dead_letter_key(runtime, queue))
        .arg(token)
        .arg(payload)
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;
    Ok(dead_lettered == 1)
}

async fn complete_successful_job_redis(
    runtime: &RedisRuntime,
    queue: &QueueId,
    token: &str,
    default_queue: &QueueId,
    effects: SuccessfulJobEffects,
) -> Result<SuccessfulJobCompletion> {
    let chain_token = effects
        .chain
        .as_ref()
        .map(|job| job.token.as_str())
        .unwrap_or("");
    let chain_payload = effects
        .chain
        .as_ref()
        .map(|job| job.payload.as_str())
        .unwrap_or("");
    let chain_queue = effects
        .chain
        .as_ref()
        .map(|job| job.queue.as_str())
        .unwrap_or("");
    let batch_id = effects.batch_id.as_deref().unwrap_or("");
    let batch_callback_token = effects.batch_callback_token.as_deref().unwrap_or("");

    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let result: Vec<i64> = ::redis::cmd("EVAL")
        .arg(COMPLETE_SUCCESSFUL_JOB_SCRIPT)
        .arg(2)
        .arg(leased_key(runtime, queue))
        .arg(payload_key(runtime, queue))
        .arg(&runtime.namespace)
        .arg(token)
        .arg(chain_token)
        .arg(chain_payload)
        .arg(chain_queue)
        .arg(batch_id)
        .arg(default_queue.as_str())
        .arg(batch_callback_token)
        .query_async(&mut conn)
        .await
        .map_err(Error::other)?;

    let lease_released = result.first().copied().unwrap_or(0) == 1;
    let batch = if result.get(2).copied().unwrap_or(0) == 1 {
        Some(BatchCompletion {
            completed: result.get(3).copied().unwrap_or(0).max(0) as u64,
            total: result.get(4).copied().unwrap_or(0).max(0) as u64,
            on_complete_enqueued: result.get(5).copied().unwrap_or(0) == 1,
        })
    } else {
        None
    };

    Ok(SuccessfulJobCompletion {
        lease_released,
        chain_enqueued: result.get(1).copied().unwrap_or(0) == 1,
        batch,
    })
}

async fn enqueue_job_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
) -> Result<()> {
    let mut payloads = runtime.payloads.lock().await;
    payloads.insert(token.to_string(), payload.to_string());
    drop(payloads);

    let mut ready = runtime.ready_queues.lock().await;
    ready
        .entry(queue.clone())
        .or_default()
        .push_back(token.to_string());
    drop(ready);
    runtime.notify.notify_waiters();
    Ok(())
}

async fn schedule_job_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
    run_at_millis: i64,
) -> Result<()> {
    let mut payloads = runtime.payloads.lock().await;
    payloads.insert(token.to_string(), payload.to_string());
    drop(payloads);

    let mut scheduled = runtime.scheduled_jobs.lock().await;
    scheduled
        .entry(queue.clone())
        .or_default()
        .push(ScheduledJobToken {
            run_at_millis,
            token: token.to_string(),
        });
    if let Some(items) = scheduled.get_mut(queue) {
        items.sort_by_key(|item| item.run_at_millis);
    }
    drop(scheduled);
    runtime.notify.notify_waiters();
    Ok(())
}

async fn promote_due_jobs_memory(
    runtime: &MemoryRuntime,
    queues: &[QueueId],
    now_millis: i64,
    limit: usize,
) -> Result<usize> {
    let mut moved = 0usize;
    let payloads = runtime.payloads.lock().await;
    let mut scheduled = runtime.scheduled_jobs.lock().await;
    let mut ready = runtime.ready_queues.lock().await;
    for queue in queues {
        if moved >= limit {
            break;
        }
        let mut due = Vec::new();
        let mut pending = Vec::new();
        for item in scheduled.remove(queue).unwrap_or_default() {
            if item.run_at_millis <= now_millis && moved + due.len() < limit {
                due.push(item.token);
            } else {
                pending.push(item);
            }
        }
        if !pending.is_empty() {
            scheduled.insert(queue.clone(), pending);
        }
        if !due.is_empty() {
            let queue_items = ready.entry(queue.clone()).or_default();
            for token in due {
                if !payloads.contains_key(&token) {
                    continue;
                }
                queue_items.push_back(token);
                moved += 1;
            }
        }
    }
    drop(ready);
    drop(scheduled);
    drop(payloads);
    if moved > 0 {
        runtime.notify.notify_waiters();
    }
    Ok(moved)
}

async fn requeue_expired_jobs_memory(
    runtime: &MemoryRuntime,
    queues: &[QueueId],
    now_millis: i64,
    limit: usize,
) -> Result<usize> {
    let mut moved = 0usize;
    let mut leased = runtime.leased_jobs.lock().await;
    let payloads = runtime.payloads.lock().await;
    let mut ready = runtime.ready_queues.lock().await;
    for queue in queues {
        if moved >= limit {
            break;
        }
        let mut expired = Vec::new();
        let mut active = Vec::new();
        for item in leased.remove(queue).unwrap_or_default() {
            if item.expires_at_millis <= now_millis && moved + expired.len() < limit {
                expired.push(item.token);
            } else {
                active.push(item);
            }
        }
        if !active.is_empty() {
            leased.insert(queue.clone(), active);
        }
        if !expired.is_empty() {
            let queue_items = ready.entry(queue.clone()).or_default();
            for token in expired {
                if !payloads.contains_key(&token) {
                    continue;
                }
                queue_items.push_back(token);
                moved += 1;
            }
        }
    }
    drop(ready);
    drop(payloads);
    drop(leased);
    if moved > 0 {
        runtime.notify.notify_waiters();
    }
    Ok(moved)
}

async fn claim_job_memory(
    runtime: &MemoryRuntime,
    queues: &[QueueId],
    lease_ttl: Duration,
) -> Result<Option<ClaimedJobLease>> {
    let mut scanned = 0usize;
    loop {
        if scanned >= CLAIM_STALE_TOKEN_SCAN_LIMIT {
            return Ok(None);
        }

        let mut ready = runtime.ready_queues.lock().await;
        let mut found = None;
        for queue in queues {
            if let Some(items) = ready.get_mut(queue) {
                if let Some(token) = items.pop_front() {
                    found = Some((queue.clone(), token));
                    break;
                }
            }
        }
        drop(ready);

        let Some((queue, token)) = found else {
            return Ok(None);
        };

        scanned += 1;
        let Some(payload) = runtime.payloads.lock().await.get(&token).cloned() else {
            continue;
        };

        let mut leased = runtime.leased_jobs.lock().await;
        if leased
            .get(&queue)
            .map(|items| items.iter().any(|item| item.token == token))
            .unwrap_or(false)
        {
            continue;
        }

        leased
            .entry(queue.clone())
            .or_default()
            .push(LeasedJobToken {
                expires_at_millis: expires_at(lease_ttl),
                token: token.clone(),
            });
        drop(leased);

        return Ok(Some(ClaimedJobLease {
            queue,
            token,
            payload,
        }));
    }
}

async fn renew_job_lease_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    lease_ttl: Duration,
) -> Result<bool> {
    let mut leased = runtime.leased_jobs.lock().await;
    if let Some(items) = leased.get_mut(queue) {
        for item in items {
            if item.token == token {
                item.expires_at_millis = expires_at(lease_ttl);
                return Ok(true);
            }
        }
    }
    Ok(false)
}

async fn retry_job_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    new_token: &str,
    payload: &str,
    run_at_millis: i64,
) -> Result<bool> {
    let removed = ack_like_memory(runtime, queue, token).await?;
    if !removed {
        return Ok(false);
    }

    let mut payloads = runtime.payloads.lock().await;
    payloads.insert(new_token.to_string(), payload.to_string());
    drop(payloads);

    let mut scheduled = runtime.scheduled_jobs.lock().await;
    scheduled
        .entry(queue.clone())
        .or_default()
        .push(ScheduledJobToken {
            run_at_millis,
            token: new_token.to_string(),
        });
    if let Some(items) = scheduled.get_mut(queue) {
        items.sort_by_key(|item| item.run_at_millis);
    }
    drop(scheduled);
    runtime.notify.notify_waiters();
    Ok(true)
}

async fn dead_letter_job_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    payload: &str,
) -> Result<bool> {
    let removed = ack_like_memory(runtime, queue, token).await?;
    if !removed {
        return Ok(false);
    }

    let mut dead_letters = runtime.dead_letters.lock().await;
    dead_letters
        .entry(queue.clone())
        .or_default()
        .push(payload.to_string());
    Ok(true)
}

async fn complete_successful_job_memory(
    runtime: &MemoryRuntime,
    queue: &QueueId,
    token: &str,
    default_queue: &QueueId,
    effects: SuccessfulJobEffects,
) -> Result<SuccessfulJobCompletion> {
    let mut leased = runtime.leased_jobs.lock().await;
    let mut released = false;
    if let Some(items) = leased.get_mut(queue) {
        let before = items.len();
        items.retain(|item| item.token != token);
        released = items.len() != before;
    }
    if !released {
        return Ok(SuccessfulJobCompletion {
            lease_released: false,
            chain_enqueued: false,
            batch: None,
        });
    }

    let mut payloads = runtime.payloads.lock().await;
    payloads.remove(token);

    let mut batch = None;
    let mut callback_job = None;
    if let Some(batch_id) = effects.batch_id.as_deref() {
        let mut batches = runtime.batches.lock().await;
        if let Some(meta) = batches.get_mut(batch_id) {
            meta.completed += 1;
            let should_enqueue_callback = meta.total > 0
                && meta.completed >= meta.total
                && meta.on_complete_job.is_some()
                && !meta.on_complete_dispatched;
            if should_enqueue_callback {
                if let (Some(payload), Some(token)) = (
                    meta.on_complete_job.clone(),
                    effects.batch_callback_token.as_deref(),
                ) {
                    let queue = meta
                        .on_complete_queue
                        .as_deref()
                        .map(QueueId::owned)
                        .unwrap_or_else(|| default_queue.clone());
                    callback_job = Some(JobToEnqueue {
                        queue,
                        token: token.to_string(),
                        payload,
                    });
                    meta.on_complete_dispatched = true;
                }
            }
            batch = Some(BatchCompletion {
                completed: meta.completed,
                total: meta.total,
                on_complete_enqueued: callback_job.is_some(),
            });
        }
    }

    let chain_enqueued = effects.chain.is_some();
    let mut ready = runtime.ready_queues.lock().await;
    if let Some(chain) = effects.chain {
        payloads.insert(chain.token.clone(), chain.payload);
        ready.entry(chain.queue).or_default().push_back(chain.token);
    }
    if let Some(callback) = callback_job {
        payloads.insert(callback.token.clone(), callback.payload);
        ready
            .entry(callback.queue)
            .or_default()
            .push_back(callback.token);
    }
    drop(ready);
    drop(payloads);
    drop(leased);

    if chain_enqueued || batch.as_ref().is_some_and(|b| b.on_complete_enqueued) {
        runtime.notify.notify_waiters();
    }

    Ok(SuccessfulJobCompletion {
        lease_released: true,
        chain_enqueued,
        batch,
    })
}

async fn ack_like_memory(runtime: &MemoryRuntime, queue: &QueueId, token: &str) -> Result<bool> {
    let mut leased = runtime.leased_jobs.lock().await;
    let mut removed = false;
    if let Some(items) = leased.get_mut(queue) {
        let before = items.len();
        items.retain(|item| item.token != token);
        removed = items.len() != before;
    }
    drop(leased);
    if removed {
        runtime.payloads.lock().await.remove(token);
    }
    Ok(removed)
}

async fn dispatch_batch_redis(
    runtime: &RedisRuntime,
    batch_id: &str,
    on_complete_payload: Option<&str>,
    on_complete_queue: Option<&str>,
    jobs: Vec<JobToEnqueue>,
) -> Result<usize> {
    let job_count = jobs.len();
    let mut conn = runtime
        .client
        .get_multiplexed_async_connection()
        .await
        .map_err(Error::other)?;
    let mut command = ::redis::cmd("EVAL");
    command
        .arg(DISPATCH_BATCH_SCRIPT)
        .arg(0)
        .arg(&runtime.namespace)
        .arg(batch_id)
        .arg(job_count as i64)
        .arg(on_complete_payload.unwrap_or(""))
        .arg(on_complete_queue.unwrap_or(""))
        .arg(job_count as i64);
    for job in jobs {
        command
            .arg(job.queue.as_str())
            .arg(job.token)
            .arg(job.payload);
    }
    let enqueued: i64 = command.query_async(&mut conn).await.map_err(Error::other)?;
    Ok(enqueued.max(0) as usize)
}

async fn dispatch_batch_memory(
    runtime: &MemoryRuntime,
    batch_id: &str,
    on_complete_payload: Option<&str>,
    on_complete_queue: Option<&str>,
    jobs: Vec<JobToEnqueue>,
) -> Result<usize> {
    use crate::support::runtime::MemoryBatchMeta;
    let total = jobs.len();
    let mut payloads = runtime.payloads.lock().await;
    let mut batches = runtime.batches.lock().await;
    let mut ready = runtime.ready_queues.lock().await;
    batches.insert(
        batch_id.to_string(),
        MemoryBatchMeta {
            total: total as u64,
            completed: 0,
            on_complete_job: on_complete_payload.map(|s| s.to_string()),
            on_complete_queue: on_complete_queue.map(|s| s.to_string()),
            on_complete_dispatched: false,
        },
    );
    for job in jobs {
        payloads.insert(job.token.clone(), job.payload);
        ready.entry(job.queue).or_default().push_back(job.token);
    }
    drop(ready);
    drop(batches);
    drop(payloads);

    if total > 0 {
        runtime.notify.notify_waiters();
    }

    Ok(total)
}

#[cfg(test)]
async fn dead_letters_memory(runtime: &MemoryRuntime, queue: &QueueId) -> Result<Vec<String>> {
    let dead_letters = runtime.dead_letters.lock().await;
    Ok(dead_letters.get(queue).cloned().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{JobToEnqueue, RuntimeBackend, SuccessfulJobEffects};
    use crate::support::runtime::{LeasedJobToken, MemoryRuntime, RedisRuntime, ScheduledJobToken};
    use crate::support::QueueId;

    fn memory_runtime(backend: &RuntimeBackend) -> Arc<MemoryRuntime> {
        match backend {
            RuntimeBackend::Memory(runtime) => Arc::clone(runtime),
            RuntimeBackend::Redis(_) => unreachable!("test backend should use memory runtime"),
        }
    }

    fn redis_runtime(backend: &RuntimeBackend) -> &RedisRuntime {
        match backend {
            RuntimeBackend::Redis(runtime) => runtime,
            RuntimeBackend::Memory(_) => unreachable!("test backend should use redis runtime"),
        }
    }

    async fn redis_backend(test_name: &str) -> Option<RuntimeBackend> {
        let url = std::env::var("FOUNDRY_REDIS_URL")
            .or_else(|_| std::env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://127.0.0.1:6379/15".to_string());
        let client = match ::redis::Client::open(url.as_str()) {
            Ok(client) => client,
            Err(error) => {
                eprintln!("skipping redis backend test `{test_name}`: {error}");
                return None;
            }
        };
        let namespace = format!(
            "foundry-tests:{}:{}:{}",
            test_name,
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        );
        let backend = RuntimeBackend::Redis(RedisRuntime { client, namespace });
        if let Err(error) = redis_ping(&backend).await {
            eprintln!("skipping redis backend test `{test_name}`: {error}");
            return None;
        }
        cleanup_redis_backend(&backend).await;
        Some(backend)
    }

    async fn redis_ping(backend: &RuntimeBackend) -> std::result::Result<(), String> {
        let runtime = redis_runtime(backend);
        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|error| error.to_string())?;
        let _: String = ::redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn cleanup_redis_backend(backend: &RuntimeBackend) {
        let runtime = redis_runtime(backend);
        let Ok(mut conn) = runtime.client.get_multiplexed_async_connection().await else {
            return;
        };
        let keys: Vec<String> = ::redis::cmd("KEYS")
            .arg(format!("{}:*", runtime.namespace))
            .query_async(&mut conn)
            .await
            .unwrap_or_default();
        if !keys.is_empty() {
            let _: std::result::Result<i64, _> =
                ::redis::cmd("DEL").arg(keys).query_async(&mut conn).await;
        }
    }

    #[tokio::test]
    async fn memory_backend_claims_and_completes_leased_jobs() {
        let backend = RuntimeBackend::memory("job-backend-ack");
        let queue = QueueId::new("default");
        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"ok\"}")
            .await
            .unwrap();

        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.queue, queue);
        assert_eq!(claimed.token, "token-1");

        let completed = backend
            .complete_successful_job(&queue, "token-1", &queue, SuccessfulJobEffects::default())
            .await
            .unwrap();
        assert!(completed.lease_released);
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn redis_backend_claim_fences_stale_and_duplicate_ready_tokens() {
        let Some(backend) = redis_backend("job-backend-redis-claim-fencing").await else {
            return;
        };
        let runtime = redis_runtime(&backend);
        let queue = QueueId::new("default");
        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();

        let _: i64 = ::redis::cmd("RPUSH")
            .arg(super::ready_key(runtime, &queue))
            .arg("missing-payload")
            .query_async(&mut conn)
            .await
            .unwrap();
        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"first\"}")
            .await
            .unwrap();
        let _: i64 = ::redis::cmd("RPUSH")
            .arg(super::ready_key(runtime, &queue))
            .arg("token-1")
            .query_async(&mut conn)
            .await
            .unwrap();
        backend
            .enqueue_job(&queue, "token-2", "{\"job\":\"second\"}")
            .await
            .unwrap();
        drop(conn);

        let first = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.token, "token-1");

        let second = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.token, "token-2");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());

        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let leased_tokens: Vec<String> = ::redis::cmd("ZRANGE")
            .arg(super::leased_key(runtime, &queue))
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert_eq!(leased_tokens, vec!["token-1", "token-2"]);
        cleanup_redis_backend(&backend).await;
    }

    #[tokio::test]
    async fn memory_backend_dispatch_batch_enqueues_all_jobs_with_callback_metadata() {
        let backend = RuntimeBackend::memory("job-backend-dispatch-batch");
        let queue = QueueId::new("default");
        let batch_id = "batch-atomic";

        let enqueued = backend
            .dispatch_batch(
                batch_id,
                Some("{\"job\":\"callback\"}"),
                Some(queue.as_str()),
                vec![
                    JobToEnqueue {
                        queue: queue.clone(),
                        token: "token-1".to_string(),
                        payload: "{\"job\":\"one\"}".to_string(),
                    },
                    JobToEnqueue {
                        queue: queue.clone(),
                        token: "token-2".to_string(),
                        payload: "{\"job\":\"two\"}".to_string(),
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(enqueued, 2);

        let first = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.token, "token-1");
        let first_complete = backend
            .complete_successful_job(
                &queue,
                &first.token,
                &queue,
                SuccessfulJobEffects {
                    batch_id: Some(batch_id.to_string()),
                    batch_callback_token: Some("callback-token-1".to_string()),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        assert!(first_complete.lease_released);
        assert_eq!(first_complete.batch.unwrap().completed, 1);

        let second = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.token, "token-2");
        let second_complete = backend
            .complete_successful_job(
                &queue,
                &second.token,
                &queue,
                SuccessfulJobEffects {
                    batch_id: Some(batch_id.to_string()),
                    batch_callback_token: Some("callback-token-2".to_string()),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        let batch = second_complete.batch.unwrap();
        assert_eq!(batch.completed, 2);
        assert_eq!(batch.total, 2);
        assert!(batch.on_complete_enqueued);

        let callback = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(callback.token, "callback-token-2");
        assert_eq!(callback.payload, "{\"job\":\"callback\"}");
    }

    #[tokio::test]
    async fn redis_backend_dispatch_batch_enqueues_all_jobs_with_callback_once() {
        let Some(backend) = redis_backend("job-backend-redis-dispatch-batch").await else {
            return;
        };
        let queue = QueueId::new("default");
        let batch_id = "batch-atomic";

        let enqueued = backend
            .dispatch_batch(
                batch_id,
                Some("{\"job\":\"callback\"}"),
                Some(queue.as_str()),
                vec![
                    JobToEnqueue {
                        queue: queue.clone(),
                        token: "token-1".to_string(),
                        payload: "{\"job\":\"one\"}".to_string(),
                    },
                    JobToEnqueue {
                        queue: queue.clone(),
                        token: "token-2".to_string(),
                        payload: "{\"job\":\"two\"}".to_string(),
                    },
                ],
            )
            .await
            .unwrap();
        assert_eq!(enqueued, 2);

        let first = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.token, "token-1");
        let first_complete = backend
            .complete_successful_job(
                &queue,
                &first.token,
                &queue,
                SuccessfulJobEffects {
                    batch_id: Some(batch_id.to_string()),
                    batch_callback_token: Some("callback-token-1".to_string()),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        assert!(first_complete.lease_released);
        let first_batch = first_complete.batch.unwrap();
        assert_eq!(first_batch.completed, 1);
        assert_eq!(first_batch.total, 2);
        assert!(!first_batch.on_complete_enqueued);

        let second = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.token, "token-2");
        let second_complete = backend
            .complete_successful_job(
                &queue,
                &second.token,
                &queue,
                SuccessfulJobEffects {
                    batch_id: Some(batch_id.to_string()),
                    batch_callback_token: Some("callback-token-2".to_string()),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        let second_batch = second_complete.batch.unwrap();
        assert_eq!(second_batch.completed, 2);
        assert_eq!(second_batch.total, 2);
        assert!(second_batch.on_complete_enqueued);

        let duplicate_complete = backend
            .complete_successful_job(
                &queue,
                &second.token,
                &queue,
                SuccessfulJobEffects {
                    batch_id: Some(batch_id.to_string()),
                    batch_callback_token: Some("callback-token-duplicate".to_string()),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        assert!(!duplicate_complete.lease_released);
        assert!(duplicate_complete.batch.is_none());

        let callback = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(callback.token, "callback-token-2");
        assert_eq!(callback.payload, "{\"job\":\"callback\"}");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        cleanup_redis_backend(&backend).await;
    }

    #[tokio::test]
    async fn memory_backend_claim_skips_ready_tokens_without_payload() {
        let backend = RuntimeBackend::memory("job-backend-claim-stale-ready");
        let runtime = memory_runtime(&backend);
        let queue = QueueId::new("default");

        runtime
            .ready_queues
            .lock()
            .await
            .entry(queue.clone())
            .or_default()
            .push_back("missing-payload".to_string());
        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"valid\"}")
            .await
            .unwrap();

        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.token, "token-1");

        let ready = runtime.ready_queues.lock().await;
        assert!(ready
            .get(&queue)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }

    #[tokio::test]
    async fn memory_backend_claim_skips_ready_tokens_that_are_already_leased() {
        let backend = RuntimeBackend::memory("job-backend-claim-duplicate-ready");
        let runtime = memory_runtime(&backend);
        let queue = QueueId::new("default");

        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"first\"}")
            .await
            .unwrap();
        runtime
            .ready_queues
            .lock()
            .await
            .entry(queue.clone())
            .or_default()
            .push_back("token-1".to_string());
        backend
            .enqueue_job(&queue, "token-2", "{\"job\":\"second\"}")
            .await
            .unwrap();

        let first = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.token, "token-1");

        let second = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.token, "token-2");

        let leased = runtime.leased_jobs.lock().await;
        let leased_tokens = leased
            .get(&queue)
            .map(|items| {
                items
                    .iter()
                    .map(|item| item.token.as_str())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert_eq!(leased_tokens, vec!["token-1", "token-2"]);
    }

    #[tokio::test]
    async fn memory_backend_claim_prunes_duplicate_ready_token_after_ack() {
        let backend = RuntimeBackend::memory("job-backend-claim-duplicate-after-ack");
        let runtime = memory_runtime(&backend);
        let queue = QueueId::new("default");

        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"once\"}")
            .await
            .unwrap();
        runtime
            .ready_queues
            .lock()
            .await
            .entry(queue.clone())
            .or_default()
            .push_back("token-1".to_string());

        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.token, "token-1");
        let completed = backend
            .complete_successful_job(&queue, "token-1", &queue, SuccessfulJobEffects::default())
            .await
            .unwrap();
        assert!(completed.lease_released);

        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        let ready = runtime.ready_queues.lock().await;
        assert!(ready
            .get(&queue)
            .map(|items| items.is_empty())
            .unwrap_or(true));
    }

    #[tokio::test]
    async fn memory_backend_promote_due_jobs_skips_tokens_without_payload() {
        let backend = RuntimeBackend::memory("job-backend-promote-stale-scheduled");
        let runtime = memory_runtime(&backend);
        let queue = QueueId::new("default");
        let now_millis = chrono::Utc::now().timestamp_millis();

        runtime
            .scheduled_jobs
            .lock()
            .await
            .entry(queue.clone())
            .or_default()
            .push(ScheduledJobToken {
                run_at_millis: now_millis - 1,
                token: "missing-payload".to_string(),
            });
        backend
            .schedule_job(&queue, "token-1", "{\"job\":\"valid\"}", now_millis - 1)
            .await
            .unwrap();

        let promoted = backend
            .promote_due_jobs(std::slice::from_ref(&queue), now_millis, 8)
            .await
            .unwrap();
        assert_eq!(promoted, 1);

        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.token, "token-1");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn memory_backend_requeues_expired_leases() {
        let backend = RuntimeBackend::memory("job-backend-requeue");
        let queue = QueueId::new("default");
        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"recover\"}")
            .await
            .unwrap();

        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.token, "token-1");

        tokio::time::sleep(Duration::from_millis(10)).await;
        let requeued = backend
            .requeue_expired_jobs(
                std::slice::from_ref(&queue),
                chrono::Utc::now().timestamp_millis(),
                8,
            )
            .await
            .unwrap();
        assert_eq!(requeued, 1);

        let reclaimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.token, "token-1");
    }

    #[tokio::test]
    async fn redis_backend_requeues_expired_leases_and_skips_missing_payloads() {
        let Some(backend) = redis_backend("job-backend-redis-requeue").await else {
            return;
        };
        let runtime = redis_runtime(&backend);
        let queue = QueueId::new("default");
        let now_millis = chrono::Utc::now().timestamp_millis();

        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"recover\"}")
            .await
            .unwrap();
        let claimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(claimed.token, "token-1");

        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let _: i64 = ::redis::cmd("ZADD")
            .arg(super::leased_key(runtime, &queue))
            .arg(now_millis - 1)
            .arg("token-1")
            .arg(now_millis - 1)
            .arg("missing-payload")
            .query_async(&mut conn)
            .await
            .unwrap();
        drop(conn);

        let requeued = backend
            .requeue_expired_jobs(std::slice::from_ref(&queue), now_millis, 8)
            .await
            .unwrap();
        assert_eq!(requeued, 1);

        let reclaimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.token, "token-1");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        cleanup_redis_backend(&backend).await;
    }

    #[tokio::test]
    async fn memory_backend_requeue_expired_jobs_skips_tokens_without_payload() {
        let backend = RuntimeBackend::memory("job-backend-requeue-stale-leased");
        let runtime = memory_runtime(&backend);
        let queue = QueueId::new("default");
        let now_millis = chrono::Utc::now().timestamp_millis();

        backend
            .enqueue_job(&queue, "token-1", "{\"job\":\"valid\"}")
            .await
            .unwrap();
        backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();

        let mut leased = runtime.leased_jobs.lock().await;
        let items = leased.entry(queue.clone()).or_default();
        for item in items.iter_mut() {
            if item.token == "token-1" {
                item.expires_at_millis = now_millis - 1;
            }
        }
        items.push(LeasedJobToken {
            expires_at_millis: now_millis - 1,
            token: "missing-payload".to_string(),
        });
        drop(leased);

        let requeued = backend
            .requeue_expired_jobs(std::slice::from_ref(&queue), now_millis, 8)
            .await
            .unwrap();
        assert_eq!(requeued, 1);

        let reclaimed = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.token, "token-1");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn redis_backend_retry_and_dead_letter_require_active_lease() {
        let Some(backend) = redis_backend("job-backend-redis-lease-transitions").await else {
            return;
        };
        let runtime = redis_runtime(&backend);
        let queue = QueueId::new("default");

        backend
            .enqueue_job(&queue, "retry-token", "{\"job\":\"retry\"}")
            .await
            .unwrap();
        let retry_claim = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(retry_claim.token, "retry-token");
        let completed = backend
            .complete_successful_job(
                &queue,
                &retry_claim.token,
                &queue,
                SuccessfulJobEffects::default(),
            )
            .await
            .unwrap();
        assert!(completed.lease_released);
        assert!(!backend
            .retry_job(
                &queue,
                &retry_claim.token,
                "retry-token-2",
                "{\"job\":\"retry-again\"}",
                chrono::Utc::now().timestamp_millis(),
            )
            .await
            .unwrap());
        assert_eq!(
            backend
                .promote_due_jobs(
                    std::slice::from_ref(&queue),
                    chrono::Utc::now().timestamp_millis() + 1,
                    8,
                )
                .await
                .unwrap(),
            0
        );

        backend
            .enqueue_job(&queue, "dead-token", "{\"job\":\"dead\"}")
            .await
            .unwrap();
        let dead_claim = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dead_claim.token, "dead-token");
        let completed = backend
            .complete_successful_job(
                &queue,
                &dead_claim.token,
                &queue,
                SuccessfulJobEffects::default(),
            )
            .await
            .unwrap();
        assert!(completed.lease_released);
        assert!(!backend
            .dead_letter_job(&queue, &dead_claim.token, "{\"failed\":true}")
            .await
            .unwrap());

        let mut conn = runtime
            .client
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let dead_letters: Vec<String> = ::redis::cmd("LRANGE")
            .arg(super::dead_letter_key(runtime, &queue))
            .arg(0)
            .arg(-1)
            .query_async(&mut conn)
            .await
            .unwrap();
        assert!(dead_letters.is_empty());
        cleanup_redis_backend(&backend).await;
    }

    #[tokio::test]
    async fn redis_backend_success_finalization_requires_lease_and_dispatches_chain_once() {
        let Some(backend) = redis_backend("job-backend-redis-success-finalization").await else {
            return;
        };
        let queue = QueueId::new("default");

        backend
            .enqueue_job(&queue, "parent-token", "{\"job\":\"parent\"}")
            .await
            .unwrap();
        let parent = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parent.token, "parent-token");

        let completed = backend
            .complete_successful_job(
                &queue,
                &parent.token,
                &queue,
                SuccessfulJobEffects {
                    chain: Some(JobToEnqueue {
                        queue: queue.clone(),
                        token: "chain-token".to_string(),
                        payload: "{\"job\":\"chain\"}".to_string(),
                    }),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        assert!(completed.lease_released);
        assert!(completed.chain_enqueued);

        let duplicate = backend
            .complete_successful_job(
                &queue,
                &parent.token,
                &queue,
                SuccessfulJobEffects {
                    chain: Some(JobToEnqueue {
                        queue: queue.clone(),
                        token: "chain-token-duplicate".to_string(),
                        payload: "{\"job\":\"chain-duplicate\"}".to_string(),
                    }),
                    ..SuccessfulJobEffects::default()
                },
            )
            .await
            .unwrap();
        assert!(!duplicate.lease_released);
        assert!(!duplicate.chain_enqueued);

        let chain = backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chain.token, "chain-token");
        assert_eq!(chain.payload, "{\"job\":\"chain\"}");
        assert!(backend
            .claim_job(std::slice::from_ref(&queue), Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        cleanup_redis_backend(&backend).await;
    }
}
