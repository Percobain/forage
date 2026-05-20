use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use rand::Rng;
use crate::cache::Cache;

#[derive(Debug, Clone)]
pub struct RateLimiterConfig {
    pub platform: String,
    pub min_delay: Duration,
    pub max_delay: Duration,
    pub daily_cap: usize,
}

pub struct PlatformLimiter {
    config: RateLimiterConfig,
    last_request: Mutex<Instant>,
    cache: Arc<Cache>,
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("{platform} daily cap reached ({count}/{cap} requests today). Resets at midnight UTC.")]
    DailyCapExceeded {
        platform: String,
        count: usize,
        cap: usize,
    },
}

impl PlatformLimiter {
    pub fn new(config: RateLimiterConfig, cache: Arc<Cache>) -> Self {
        Self {
            config,
            last_request: Mutex::new(Instant::now() - Duration::from_secs(60)),
            cache,
        }
    }

    pub async fn acquire(&self) -> Result<(), RateLimitError> {
        // 1. Check daily cap
        let (_, count) = self.cache.get_rate_limit_count(&self.config.platform);
        if count >= self.config.daily_cap {
            return Err(RateLimitError::DailyCapExceeded {
                platform: self.config.platform.clone(),
                count,
                cap: self.config.daily_cap,
            });
        }

        // 2. Wait for minimum delay since last request
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        let jitter_delay = {
            let mut rng = rand::thread_rng();
            let min_ms = self.config.min_delay.as_millis() as u64;
            let max_ms = self.config.max_delay.as_millis() as u64;
            Duration::from_millis(rng.gen_range(min_ms..=max_ms))
        };

        if elapsed < jitter_delay {
            let sleep_time = jitter_delay - elapsed;
            tokio::time::sleep(sleep_time).await;
        }

        // 3. Update state
        *last = Instant::now();
        self.cache.increment_rate_limit(&self.config.platform);

        Ok(())
    }

    pub fn remaining_today(&self) -> usize {
        let (_, count) = self.cache.get_rate_limit_count(&self.config.platform);
        self.config.daily_cap.saturating_sub(count)
    }
}
