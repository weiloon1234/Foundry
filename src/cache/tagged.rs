use std::future::Future;
use std::time::Duration;

use serde::{de::DeserializeOwned, Serialize};
use uuid::Uuid;

use crate::foundation::{Error, Result};
use crate::support::sha256_hex_str;

use super::CacheManager;

const INITIAL_TAG_VERSION: &str = "0";
const TAGGED_KEY_PREFIX: &str = "\u{001f}foundry:cache:tagged:";

/// A cache view whose entries are invalidated together by one or more tags.
///
/// Tag names are canonicalized by sorting and deduplicating them. Flushing a
/// tag advances its shared version instead of scanning or deleting entries.
#[derive(Clone)]
pub struct TaggedCache<'cache> {
    cache: &'cache CacheManager,
    tags: Vec<String>,
}

impl CacheManager {
    /// Create a cache view scoped to the supplied tags.
    ///
    /// Tag order and duplicate tag names do not affect the cache identity.
    /// Invalid or empty tag sets are reported when an operation is performed.
    pub fn tags<I, S>(&self, tags: I) -> TaggedCache<'_>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        TaggedCache::new(self, tags)
    }
}

impl<'cache> TaggedCache<'cache> {
    fn new<I, S>(cache: &'cache CacheManager, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut tags = tags
            .into_iter()
            .map(|tag| tag.as_ref().to_string())
            .collect::<Vec<_>>();
        tags.sort_unstable();
        tags.dedup();
        Self { cache, tags }
    }

    /// Return the canonical sorted, deduplicated tag names.
    pub fn tag_names(&self) -> &[String] {
        &self.tags
    }

    /// Get a tagged value from cache.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let resolved = self.resolve_key(key).await?;
        self.cache.get_unchecked(&resolved).await
    }

    /// Store a tagged value in cache with a TTL.
    pub async fn put<T: Serialize>(&self, key: &str, value: &T, ttl: Duration) -> Result<()> {
        let resolved = self.resolve_key(key).await?;
        self.cache.put_unchecked(&resolved, value, ttl).await
    }

    /// Get a tagged value, or compute and store it with a TTL.
    pub async fn remember<T, F, Fut>(&self, key: &str, ttl: Duration, callback: F) -> Result<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let resolved = self.resolve_key(key).await?;
        self.cache
            .remember_unchecked(&resolved, ttl, callback)
            .await
    }

    /// Remove the value stored under this exact tag set and key.
    pub async fn forget(&self, key: &str) -> Result<bool> {
        let resolved = self.resolve_key(key).await?;
        self.cache.forget_unchecked(&resolved).await
    }

    /// Invalidate every cached entry containing any of these tags.
    pub async fn flush(&self) -> Result<()> {
        self.validate_tags()?;
        let version = Uuid::now_v7().to_string();
        for tag in &self.tags {
            self.cache
                .put_control_raw(&tag_version_key(tag), &version)
                .await?;
        }
        Ok(())
    }

    async fn resolve_key(&self, key: &str) -> Result<String> {
        self.validate_tags()?;
        self.cache.validate_key(key)?;

        let mut versions = String::with_capacity(self.tags.len() * 102);
        for tag in &self.tags {
            let version_key = tag_version_key(tag);
            let version = self
                .cache
                .get_control_raw(&version_key)
                .await?
                .unwrap_or_else(|| INITIAL_TAG_VERSION.to_string());
            versions.push_str(&version_key);
            versions.push('\0');
            versions.push_str(&version);
            versions.push('\0');
        }

        Ok(format!(
            "{TAGGED_KEY_PREFIX}{}:{key}",
            sha256_hex_str(&versions)
        ))
    }

    fn validate_tags(&self) -> Result<()> {
        if self.tags.is_empty() {
            return Err(Error::message("cache tags cannot be empty"));
        }
        for tag in &self.tags {
            if tag.is_empty() {
                return Err(Error::message("cache tag cannot be empty"));
            }
            if tag.chars().any(char::is_control) {
                return Err(Error::message(
                    "cache tag cannot contain control characters",
                ));
            }
            if self.cache.config.key_max_length > 0 && tag.len() > self.cache.config.key_max_length
            {
                return Err(Error::message(format!(
                    "cache tag exceeds maximum length of {} bytes",
                    self.cache.config.key_max_length
                )));
            }
        }
        Ok(())
    }
}

fn tag_version_key(tag: &str) -> String {
    format!("tag:{}", sha256_hex_str(tag))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::CacheManager;
    use crate::cache::MemoryCacheStore;
    use crate::config::CacheConfig;

    fn shared_managers() -> (CacheManager, CacheManager) {
        let store = Arc::new(MemoryCacheStore::new(100));
        (
            CacheManager::with_config(store.clone(), CacheConfig::default(), None),
            CacheManager::with_config(store, CacheConfig::default(), None),
        )
    }

    #[tokio::test]
    async fn tag_order_and_duplicates_share_one_cache_identity() {
        let (first, second) = shared_managers();
        let source = ["users", "admins", "users"];
        let tags = first.tags(source);
        assert_eq!(
            tags.tag_names(),
            &["admins".to_string(), "users".to_string()]
        );

        tags.put("profile", &"cached", Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(
            second
                .tags(["admins", "users"])
                .get::<String>("profile")
                .await
                .unwrap(),
            Some("cached".to_string())
        );
    }

    #[tokio::test]
    async fn flushing_a_tag_invalidates_all_matching_sets_only() {
        let (first, second) = shared_managers();

        first
            .tags(["users"])
            .put("record", &"user", Duration::from_secs(60))
            .await
            .unwrap();
        first
            .tags(["admins"])
            .put("record", &"admin", Duration::from_secs(60))
            .await
            .unwrap();
        first
            .tags(["users", "admins"])
            .put("record", &"both", Duration::from_secs(60))
            .await
            .unwrap();
        first
            .put("record", &"untagged", Duration::from_secs(60))
            .await
            .unwrap();

        second.tags(["users"]).flush().await.unwrap();

        assert_eq!(
            first.tags(["users"]).get::<String>("record").await.unwrap(),
            None
        );
        assert_eq!(
            first
                .tags(["users", "admins"])
                .get::<String>("record")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            first
                .tags(["admins"])
                .get::<String>("record")
                .await
                .unwrap(),
            Some("admin".to_string())
        );
        assert_eq!(
            first.get::<String>("record").await.unwrap(),
            Some("untagged".to_string())
        );
    }

    #[tokio::test]
    async fn remember_and_forget_use_the_resolved_tag_identity() {
        let (cache, _) = shared_managers();

        let value = cache
            .tags(["users"])
            .remember("profile", Duration::from_secs(60), || async {
                Ok::<_, crate::Error>("computed".to_string())
            })
            .await
            .unwrap();
        assert_eq!(value, "computed");
        assert!(cache.tags(["users"]).forget("profile").await.unwrap());
        assert_eq!(
            cache
                .tags(["users"])
                .get::<String>("profile")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn invalid_tag_sets_return_clear_errors() {
        let (cache, _) = shared_managers();

        let empty = cache.tags(Vec::<String>::new()).flush().await.unwrap_err();
        assert_eq!(empty.to_string(), "cache tags cannot be empty");

        let invalid = cache.tags(["bad\ntag"]).flush().await.unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "cache tag cannot contain control characters"
        );
    }

    #[tokio::test]
    async fn global_memory_flush_removes_tagged_and_untagged_values() {
        let (first, second) = shared_managers();
        first
            .tags(["users"])
            .put("tagged", &"value", Duration::from_secs(60))
            .await
            .unwrap();
        first
            .put("untagged", &"value", Duration::from_secs(60))
            .await
            .unwrap();

        second.flush().await.unwrap();

        assert_eq!(
            first.tags(["users"]).get::<String>("tagged").await.unwrap(),
            None
        );
        assert_eq!(first.get::<String>("untagged").await.unwrap(), None);
    }
}
