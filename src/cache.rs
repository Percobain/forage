use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Cache {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub url: String,
    pub method: String,
    pub content: String,
    pub status_code: Option<i32>,
    pub fetched_at: i64,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(path)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cache (
                key TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                method TEXT NOT NULL,
                content TEXT NOT NULL,
                status_code INTEGER,
                fetched_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                size_bytes INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_expires ON cache(expires_at);
            CREATE INDEX IF NOT EXISTS idx_url ON cache(url);

            CREATE TABLE IF NOT EXISTS rate_limit_state (
                platform TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                count INTEGER NOT NULL DEFAULT 0,
                last_request_at INTEGER
            );
            ",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Build a cache key from method + url + optional extra params
    pub fn cache_key(method: &str, url: &str, extra: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{method}:{url}:{extra}"));
        hex::encode(hasher.finalize())
    }

    pub fn get(&self, key: &str) -> Option<CacheEntry> {
        let conn = self.conn.lock().unwrap();
        let now = now_unix();
        conn.query_row(
            "SELECT url, method, content, status_code, fetched_at
             FROM cache WHERE key = ?1 AND expires_at > ?2",
            params![key, now],
            |row| {
                Ok(CacheEntry {
                    url: row.get(0)?,
                    method: row.get(1)?,
                    content: row.get(2)?,
                    status_code: row.get(3)?,
                    fetched_at: row.get(4)?,
                })
            },
        )
        .optional()
        .unwrap_or(None)
    }

    pub fn put(
        &self,
        key: &str,
        url: &str,
        method: &str,
        content: &str,
        status_code: Option<i32>,
        ttl_seconds: u64,
    ) {
        let conn = self.conn.lock().unwrap();
        let now = now_unix();
        let expires = now + ttl_seconds as i64;
        let size = content.len() as i64;
        conn.execute(
            "INSERT OR REPLACE INTO cache (key, url, method, content, status_code, fetched_at, expires_at, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![key, url, method, content, status_code, now, expires, size],
        )
        .ok();
    }

    pub fn evict_expired(&self) -> usize {
        let conn = self.conn.lock().unwrap();
        let now = now_unix();
        conn.execute("DELETE FROM cache WHERE expires_at <= ?1", params![now])
            .unwrap_or(0)
    }

    /// Get today's request count for a platform
    pub fn get_rate_limit_count(&self, platform: &str) -> (String, usize) {
        let conn = self.conn.lock().unwrap();
        let today = today_str();
        let result = conn
            .query_row(
                "SELECT date, count FROM rate_limit_state WHERE platform = ?1",
                params![platform],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)),
            )
            .optional()
            .unwrap_or(None);

        match result {
            Some((date, count)) if date == today => (date, count),
            _ => (today, 0),
        }
    }

    /// Increment rate limit counter. Returns new count.
    pub fn increment_rate_limit(&self, platform: &str) -> usize {
        let conn = self.conn.lock().unwrap();
        let today = today_str();
        let now = now_unix();

        // Check current state
        let current = conn
            .query_row(
                "SELECT date, count FROM rate_limit_state WHERE platform = ?1",
                params![platform],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?)),
            )
            .optional()
            .unwrap_or(None);

        let new_count = match current {
            Some((date, count)) if date == today => count + 1,
            _ => 1,
        };

        conn.execute(
            "INSERT OR REPLACE INTO rate_limit_state (platform, date, count, last_request_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![platform, today, new_count, now],
        )
        .ok();

        new_count
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn today_str() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_cache() -> (Cache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_cache.db");
        let cache = Cache::open(&db_path).unwrap();
        (cache, dir)
    }

    #[test]
    fn test_put_and_get() {
        let (cache, _dir) = temp_cache();
        let key = Cache::cache_key("direct", "https://example.com", "");

        cache.put(&key, "https://example.com", "direct", "# Hello World", Some(200), 3600);

        let entry = cache.get(&key).unwrap();
        assert_eq!(entry.url, "https://example.com");
        assert_eq!(entry.method, "direct");
        assert_eq!(entry.content, "# Hello World");
        assert_eq!(entry.status_code, Some(200));
    }

    #[test]
    fn test_expired_entry_not_returned() {
        let (cache, _dir) = temp_cache();
        let key = Cache::cache_key("direct", "https://example.com", "");

        // TTL of 0 means it expires immediately
        cache.put(&key, "https://example.com", "direct", "expired content", Some(200), 0);

        // Should not be found (expires_at == fetched_at, and we check expires_at > now)
        let entry = cache.get(&key);
        assert!(entry.is_none());
    }

    #[test]
    fn test_cache_key_deterministic() {
        let k1 = Cache::cache_key("direct", "https://example.com", "");
        let k2 = Cache::cache_key("direct", "https://example.com", "");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_cache_key_differs_by_method() {
        let k1 = Cache::cache_key("direct", "https://example.com", "");
        let k2 = Cache::cache_key("jina", "https://example.com", "");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_evict_expired() {
        let (cache, _dir) = temp_cache();

        // Insert expired entry
        {
            let conn = cache.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO cache (key, url, method, content, status_code, fetched_at, expires_at, size_bytes)
                 VALUES ('old', 'http://old.com', 'direct', 'old', 200, 1000, 1001, 3)",
                [],
            ).unwrap();
        }

        // Insert valid entry
        cache.put("new", "http://new.com", "direct", "new content", Some(200), 3600);

        let evicted = cache.evict_expired();
        assert_eq!(evicted, 1);

        assert!(cache.get("old").is_none());
        assert!(cache.get("new").is_some());
    }

    #[test]
    fn test_rate_limit_counter() {
        let (cache, _dir) = temp_cache();

        let (_, count) = cache.get_rate_limit_count("linkedin");
        assert_eq!(count, 0);

        let c1 = cache.increment_rate_limit("linkedin");
        assert_eq!(c1, 1);

        let c2 = cache.increment_rate_limit("linkedin");
        assert_eq!(c2, 2);

        let (_, count) = cache.get_rate_limit_count("linkedin");
        assert_eq!(count, 2);
    }

    #[test]
    fn test_overwrite_entry() {
        let (cache, _dir) = temp_cache();
        let key = "test_key";

        cache.put(key, "https://example.com", "direct", "v1", Some(200), 3600);
        cache.put(key, "https://example.com", "direct", "v2", Some(200), 3600);

        let entry = cache.get(key).unwrap();
        assert_eq!(entry.content, "v2");
    }
}
