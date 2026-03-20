use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::models::{ApiResponse, Game, Team};

const BASE_URL: &str = "https://api.balldontlie.io/v1";

/// Rate limit: free tier allows 5 requests per minute.
/// We use a global rate limiter to ensure all API calls (teams + games)
/// share this budget regardless of which code path initiates them.
const RATE_LIMIT_MAX_REQUESTS: u32 = 5;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(61);

/// How many game date-range requests to fire concurrently within a single
/// rate-limit window. Set to 4 so there's 1 slot left for get_teams() if
/// it was recently called in the same window.
const RATE_LIMIT_BATCH_SIZE: usize = 4;

/// Maximum results per page (API max is 100).
const PER_PAGE: u32 = 100;

/// Maximum number of retry attempts when the API returns 429 Too Many Requests.
const MAX_RETRIES: u32 = 5;

/// Tracks how many requests have been sent in the current rate-limit window
/// so we can sleep before exceeding the limit.
struct RateLimiterState {
    /// Number of requests sent in the current window.
    requests_in_window: u32,
    /// When the current rate-limit window started.
    window_start: tokio::time::Instant,
}

/// Global rate limiter that ensures all API calls (teams, games, retries)
/// stay within the free-tier limit of 5 requests per 60 seconds.
///
/// Every request must call `acquire()` before hitting the API. If the
/// window's budget is exhausted, `acquire()` sleeps until the window resets.
struct RateLimiter {
    state: Mutex<RateLimiterState>,
    /// When false, `acquire()` is a no-op. Used in tests to avoid waits.
    enabled: bool,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            state: Mutex::new(RateLimiterState {
                requests_in_window: 0,
                window_start: tokio::time::Instant::now(),
            }),
            enabled: true,
        }
    }

    /// Create a rate limiter that never blocks. Used in tests to avoid
    /// 61-second waits between batches.
    #[cfg(test)]
    fn disabled() -> Self {
        Self {
            state: Mutex::new(RateLimiterState {
                requests_in_window: 0,
                window_start: tokio::time::Instant::now(),
            }),
            enabled: false,
        }
    }

    /// Wait until a request slot is available, then consume one slot.
    ///
    /// If all slots in the current window are used, this sleeps until the
    /// window resets. The mutex ensures only one caller checks/updates the
    /// state at a time, preventing thundering-herd issues.
    async fn acquire(&self) {
        if !self.enabled {
            return;
        }

        let mut state = self.state.lock().await;

        // If the window has elapsed, reset the counter
        let elapsed = state.window_start.elapsed();
        if elapsed >= RATE_LIMIT_WINDOW {
            state.requests_in_window = 0;
            state.window_start = tokio::time::Instant::now();
        }

        // If we've used all slots in this window, wait for the window to reset
        if state.requests_in_window >= RATE_LIMIT_MAX_REQUESTS {
            let remaining = RATE_LIMIT_WINDOW.saturating_sub(state.window_start.elapsed());
            if !remaining.is_zero() {
                info!(
                    "Rate limiter: {}/{} requests used, waiting {:.0}s for window reset",
                    state.requests_in_window,
                    RATE_LIMIT_MAX_REQUESTS,
                    remaining.as_secs_f64()
                );
                // Drop the lock while sleeping so other tasks aren't blocked
                // from queueing up behind us. We re-acquire and re-check after.
                drop(state);
                tokio::time::sleep(remaining).await;
                state = self.state.lock().await;

                // Reset after sleeping
                state.requests_in_window = 0;
                state.window_start = tokio::time::Instant::now();
            } else {
                // Window just expired while we were checking, reset now
                state.requests_in_window = 0;
                state.window_start = tokio::time::Instant::now();
            }
        }

        state.requests_in_window += 1;
        debug!(
            "Rate limiter: slot {}/{} acquired",
            state.requests_in_window, RATE_LIMIT_MAX_REQUESTS
        );
    }
}

/// HTTP client for the balldontlie NBA API.
///
/// All requests go through a shared [`RateLimiter`] so that concurrent
/// callers (pre-warm, slash commands, cron scheduler) never exceed the
/// free-tier limit of 5 requests per minute.
pub struct BallDontLieClient {
    client: Client,
    api_key: String,
    base_url: String,
    rate_limiter: Arc<RateLimiter>,
}

impl BallDontLieClient {
    /// Create a new API client with the given API key.
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            api_key,
            base_url: BASE_URL.to_string(),
            rate_limiter: Arc::new(RateLimiter::new()),
        })
    }

    /// Create a new API client pointing at a custom base URL with rate
    /// limiting disabled. Used in tests to point at a mock server without
    /// incurring 61-second waits between request batches.
    #[cfg(test)]
    pub fn with_base_url(api_key: String, base_url: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self {
            client,
            api_key,
            base_url,
            rate_limiter: Arc::new(RateLimiter::disabled()),
        })
    }

    /// Fetch all 30 NBA teams, retrying on 429 with exponential backoff.
    pub async fn get_teams(&self) -> Result<Vec<Team>> {
        let url = format!("{}/teams", self.base_url);
        let mut attempt = 0u32;

        loop {
            self.rate_limiter.acquire().await;

            let resp = self
                .client
                .get(&url)
                .header("Authorization", &self.api_key)
                .send()
                .await
                .context("Failed to fetch teams")?;

            let status = resp.status();

            // Retry on 429 with exponential backoff
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                attempt += 1;
                if attempt > MAX_RETRIES {
                    anyhow::bail!("Teams API rate limited after {attempt} retries");
                }
                let backoff =
                    Duration::from_secs(RATE_LIMIT_WINDOW.as_secs() * 2u64.pow(attempt - 1));
                warn!("Teams API rate limited (429), retry {attempt}/{MAX_RETRIES} in {backoff:?}");
                tokio::time::sleep(backoff).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Teams API returned {status}: {body}");
            }

            let api_resp: ApiResponse<Team> = resp
                .json()
                .await
                .context("Failed to parse teams response")?;

            debug!("Fetched {} teams", api_resp.data.len());
            return Ok(api_resp.data);
        }
    }

    /// Fetch all regular season games for a given season using parallel
    /// date-range fetching to minimize wall-clock time.
    ///
    /// Splits the season into date ranges and fetches them concurrently,
    /// staying within the 5 req/min rate limit via the global rate limiter.
    pub async fn get_season_games(&self, season: u32) -> Result<Vec<Game>> {
        let ranges = season_date_ranges(season);
        self.get_games_parallel(&ranges, season).await
    }

    /// Fetch regular season games starting from a specific date.
    /// Used for incremental cache updates (typically 0-1 pages).
    pub async fn get_games_since(
        &self,
        season: u32,
        start_date: Option<&str>,
    ) -> Result<Vec<Game>> {
        match start_date {
            Some(date) => {
                let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
                let ranges = vec![(date.to_string(), today)];
                self.get_games_parallel(&ranges, season).await
            }
            None => self.get_season_games(season).await,
        }
    }

    /// Fetch games across multiple date ranges in parallel, respecting
    /// the rate limit via the global rate limiter.
    ///
    /// Each date range is paginated independently. We gather up to
    /// RATE_LIMIT_BATCH_SIZE pages concurrently per iteration, then handle
    /// any remaining pages in subsequent iterations.
    async fn get_games_parallel(
        &self,
        ranges: &[(String, String)],
        season: u32,
    ) -> Result<Vec<Game>> {
        let mut all_games: Vec<Game> = Vec::new();

        // Each range needs its own pagination state
        struct RangeState {
            start_date: String,
            end_date: String,
            cursor: Option<u64>,
            done: bool,
        }

        let mut states: Vec<RangeState> = ranges
            .iter()
            .map(|(start, end)| RangeState {
                start_date: start.clone(),
                end_date: end.clone(),
                cursor: None,
                done: false,
            })
            .collect();

        let mut total_requests = 0u32;

        loop {
            // Collect pending work as owned data to avoid borrowing states
            let pending: Vec<(usize, String, String, Option<u64>)> = states
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.done)
                .map(|(i, s)| (i, s.start_date.clone(), s.end_date.clone(), s.cursor))
                .collect();

            if pending.is_empty() {
                break;
            }

            // Process in batches of RATE_LIMIT_BATCH_SIZE
            for batch in pending.chunks(RATE_LIMIT_BATCH_SIZE) {
                let batch_size = batch.len();

                // Fire all requests in this batch concurrently.
                // Each fetch_games_page call acquires its own rate-limit slot.
                let futures: Vec<_> = batch
                    .iter()
                    .map(|(_, start, end, cursor)| {
                        self.fetch_games_page(season, start, end, *cursor)
                    })
                    .collect();

                let results = futures::future::join_all(futures).await;

                // Process results and update states
                for (result, (idx, _, _, _)) in results.into_iter().zip(batch.iter()) {
                    let (games, next_cursor) = result?;
                    all_games.extend(games);

                    match next_cursor {
                        Some(c) => states[*idx].cursor = Some(c),
                        None => states[*idx].done = true,
                    }
                }

                total_requests += batch_size as u32;

                let still_pending = states.iter().any(|s| !s.done);
                if still_pending {
                    info!(
                        "Fetched {batch_size} pages ({} games so far), continuing...",
                        all_games.len()
                    );
                }
            }
        }

        info!(
            "Fetched {total_requests} pages for season {season}: {} total games",
            all_games.len()
        );
        Ok(all_games)
    }

    /// Fetch a single page of games for a date range, retrying on 429
    /// with exponential backoff.
    ///
    /// Returns the games and the next cursor (None if no more pages).
    async fn fetch_games_page(
        &self,
        season: u32,
        start_date: &str,
        end_date: &str,
        cursor: Option<u64>,
    ) -> Result<(Vec<Game>, Option<u64>)> {
        let mut attempt = 0u32;

        loop {
            self.rate_limiter.acquire().await;

            let mut request = self
                .client
                .get(format!("{}/games", self.base_url))
                .header("Authorization", &self.api_key)
                .query(&[
                    ("seasons[]", season.to_string()),
                    ("postseason", "false".to_string()),
                    ("per_page", PER_PAGE.to_string()),
                    ("start_date", start_date.to_string()),
                    ("end_date", end_date.to_string()),
                ]);

            if let Some(c) = cursor {
                request = request.query(&[("cursor", c.to_string())]);
            }

            let resp = request
                .send()
                .await
                .with_context(|| format!("Failed to fetch games ({start_date} to {end_date})"))?;

            let status = resp.status();

            // Retry on 429 with exponential backoff: 61s, 122s, 244s, ...
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                attempt += 1;
                if attempt > MAX_RETRIES {
                    anyhow::bail!(
                        "Games API rate limited after {attempt} retries ({start_date} to {end_date})"
                    );
                }
                let backoff =
                    Duration::from_secs(RATE_LIMIT_WINDOW.as_secs() * 2u64.pow(attempt - 1));
                warn!(
                    "Rate limited (429), retry {attempt}/{MAX_RETRIES} in {backoff:?} ({start_date} to {end_date})"
                );
                tokio::time::sleep(backoff).await;
                continue;
            }

            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                anyhow::bail!("Games API returned {status} ({start_date} to {end_date}): {body}");
            }

            let api_resp: ApiResponse<Game> = resp.json().await.with_context(|| {
                format!("Failed to parse games response ({start_date} to {end_date})")
            })?;

            let next_cursor = api_resp.meta.and_then(|m| m.next_cursor);
            let count = api_resp.data.len();
            debug!(
                "Fetched {count} games for {start_date}..{end_date} (cursor: {cursor:?}, next: {next_cursor:?})"
            );

            return Ok((api_resp.data, next_cursor));
        }
    }
}

/// Split an NBA season into date ranges for parallel fetching.
///
/// The NBA regular season runs from mid-October to mid-April (~180 game days).
/// We end at April 30 to capture any schedule extensions while avoiding months
/// of empty postseason/offseason results. The `postseason=false` query param
/// filters out playoff games, but narrowing the date range reduces the total
/// number of API pages returned.
///
/// We split this into RATE_LIMIT_BATCH_SIZE ranges so we can fetch the first
/// page of each concurrently within a single rate-limit window.
#[cfg_attr(test, allow(dead_code))]
fn season_date_ranges(season: u32) -> Vec<(String, String)> {
    let start = NaiveDate::from_ymd_opt(season as i32, 10, 1).expect("valid season start date");
    let end = NaiveDate::from_ymd_opt((season + 1) as i32, 4, 30).expect("valid season end date");

    let total_days = (end - start).num_days();
    let chunk_days = total_days / RATE_LIMIT_BATCH_SIZE as i64;

    let mut ranges = Vec::with_capacity(RATE_LIMIT_BATCH_SIZE);
    let mut chunk_start = start;

    for i in 0..RATE_LIMIT_BATCH_SIZE {
        let chunk_end = if i == RATE_LIMIT_BATCH_SIZE - 1 {
            end
        } else {
            chunk_start + chrono::Duration::days(chunk_days - 1)
        };

        ranges.push((
            chunk_start.format("%Y-%m-%d").to_string(),
            chunk_end.format("%Y-%m-%d").to_string(),
        ));

        chunk_start = chunk_end + chrono::Duration::days(1);
    }

    debug!("Season {season} split into {} date ranges:", ranges.len());
    for (s, e) in &ranges {
        debug!("  {s} to {e}");
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── season_date_ranges ──────────────────────────────────────────

    #[test]
    fn season_date_ranges_returns_correct_count() {
        let ranges = season_date_ranges(2025);
        assert_eq!(ranges.len(), RATE_LIMIT_BATCH_SIZE);
    }

    #[test]
    fn season_date_ranges_starts_october_first() {
        let ranges = season_date_ranges(2025);
        assert_eq!(ranges[0].0, "2025-10-01");
    }

    #[test]
    fn season_date_ranges_ends_april_thirtieth() {
        let ranges = season_date_ranges(2025);
        let last = &ranges[ranges.len() - 1];
        assert_eq!(last.1, "2026-04-30");
    }

    #[test]
    fn season_date_ranges_are_contiguous() {
        let ranges = season_date_ranges(2025);
        for i in 0..ranges.len() - 1 {
            let end = NaiveDate::parse_from_str(&ranges[i].1, "%Y-%m-%d").unwrap();
            let next_start = NaiveDate::parse_from_str(&ranges[i + 1].0, "%Y-%m-%d").unwrap();
            assert_eq!(
                next_start - end,
                chrono::Duration::days(1),
                "Gap between range {} end ({}) and range {} start ({})",
                i,
                ranges[i].1,
                i + 1,
                ranges[i + 1].0,
            );
        }
    }

    #[test]
    fn season_date_ranges_no_overlaps() {
        let ranges = season_date_ranges(2025);
        for (start, end) in &ranges {
            let s = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
            let e = NaiveDate::parse_from_str(end, "%Y-%m-%d").unwrap();
            assert!(s <= e, "Range start {start} is after end {end}");
        }
    }

    #[test]
    fn season_date_ranges_different_seasons() {
        for season in [2020, 2023, 2025, 2030] {
            let ranges = season_date_ranges(season);
            assert_eq!(ranges.len(), RATE_LIMIT_BATCH_SIZE);
            assert_eq!(ranges[0].0, format!("{season}-10-01"));
            assert_eq!(ranges.last().unwrap().1, format!("{}-04-30", season + 1));
        }
    }

    #[test]
    fn season_date_ranges_covers_full_regular_season_window() {
        let ranges = season_date_ranges(2025);
        let first_start = NaiveDate::parse_from_str(&ranges[0].0, "%Y-%m-%d").unwrap();
        let last_end = NaiveDate::parse_from_str(&ranges.last().unwrap().1, "%Y-%m-%d").unwrap();
        let total_days = (last_end - first_start).num_days();
        // Oct 1 to Apr 30 = 211 days
        assert_eq!(total_days, 211);
    }

    // ── RateLimiter ─────────────────────────────────────────────────

    #[tokio::test]
    async fn rate_limiter_allows_up_to_max_requests_immediately() {
        let limiter = RateLimiter::new();

        let start = tokio::time::Instant::now();
        for _ in 0..RATE_LIMIT_MAX_REQUESTS {
            limiter.acquire().await;
        }
        let elapsed = start.elapsed();

        // All 5 should be near-instant (well under 1 second)
        assert!(
            elapsed < Duration::from_millis(100),
            "Expected near-instant, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn rate_limiter_blocks_after_max_requests() {
        tokio::time::pause();
        let limiter = RateLimiter::new();

        // Use all slots
        for _ in 0..RATE_LIMIT_MAX_REQUESTS {
            limiter.acquire().await;
        }

        // The next acquire should block until the window resets
        let start = tokio::time::Instant::now();
        limiter.acquire().await;
        let elapsed = start.elapsed();

        // Should have waited approximately RATE_LIMIT_WINDOW
        assert!(
            elapsed >= RATE_LIMIT_WINDOW,
            "Expected to wait ~{:?}, only waited {:?}",
            RATE_LIMIT_WINDOW,
            elapsed
        );
    }

    #[tokio::test]
    async fn rate_limiter_resets_after_window_elapses() {
        tokio::time::pause();
        let limiter = RateLimiter::new();

        // Use all slots
        for _ in 0..RATE_LIMIT_MAX_REQUESTS {
            limiter.acquire().await;
        }

        // Advance past the window
        tokio::time::advance(RATE_LIMIT_WINDOW + Duration::from_secs(1)).await;

        // Should be able to acquire again immediately
        let start = tokio::time::Instant::now();
        limiter.acquire().await;
        let elapsed = start.elapsed();

        assert!(
            elapsed < Duration::from_millis(10),
            "Expected near-instant after window reset, took {:?}",
            elapsed
        );
    }

    #[tokio::test]
    async fn rate_limiter_concurrent_acquires_serialize() {
        tokio::time::pause();
        let limiter = Arc::new(RateLimiter::new());

        // Spawn RATE_LIMIT_MAX_REQUESTS + 2 concurrent acquires
        let total = RATE_LIMIT_MAX_REQUESTS + 2;
        let mut handles = Vec::new();

        for _ in 0..total {
            let limiter = limiter.clone();
            handles.push(tokio::spawn(async move {
                limiter.acquire().await;
                tokio::time::Instant::now()
            }));
        }

        let mut timestamps = Vec::new();
        for handle in handles {
            timestamps.push(handle.await.unwrap());
        }

        // At least 2 acquires should have waited for the next window
        timestamps.sort();
        let first = timestamps[0];
        let last = timestamps[timestamps.len() - 1];
        assert!(
            last - first >= RATE_LIMIT_WINDOW,
            "Expected at least one window wait for {} acquires, but spread was {:?}",
            total,
            last - first
        );
    }

    // ── Constants ───────────────────────────────────────────────────

    #[test]
    fn rate_limit_batch_size_leaves_room_for_teams_request() {
        // Batch size must be strictly less than max requests so that
        // get_teams() can use a slot in the same window.
        assert!(
            RATE_LIMIT_BATCH_SIZE < RATE_LIMIT_MAX_REQUESTS as usize,
            "RATE_LIMIT_BATCH_SIZE ({}) must be < RATE_LIMIT_MAX_REQUESTS ({})",
            RATE_LIMIT_BATCH_SIZE,
            RATE_LIMIT_MAX_REQUESTS,
        );
    }

    #[test]
    fn rate_limit_window_exceeds_one_minute() {
        // The API rate limit is per minute, so our window must be > 60s
        assert!(
            RATE_LIMIT_WINDOW > Duration::from_secs(60),
            "RATE_LIMIT_WINDOW must exceed 60 seconds"
        );
    }

    #[test]
    fn per_page_is_api_maximum() {
        assert_eq!(PER_PAGE, 100, "PER_PAGE should be the API's maximum of 100");
    }

    #[test]
    fn max_retries_is_reasonable() {
        assert!(
            MAX_RETRIES >= 3 && MAX_RETRIES <= 10,
            "MAX_RETRIES ({}) should be between 3 and 10",
            MAX_RETRIES
        );
    }

    // ── Exponential backoff calculation ─────────────────────────────

    #[test]
    fn backoff_is_exponential_not_linear() {
        let window_secs = RATE_LIMIT_WINDOW.as_secs();

        let backoff_1 = window_secs * 2u64.pow(0); // attempt 1
        let backoff_2 = window_secs * 2u64.pow(1); // attempt 2
        let backoff_3 = window_secs * 2u64.pow(2); // attempt 3

        assert_eq!(backoff_1, 61, "First backoff should be 61s");
        assert_eq!(backoff_2, 122, "Second backoff should be 122s");
        assert_eq!(backoff_3, 244, "Third backoff should be 244s");
        // Verify it's truly exponential: each step doubles
        assert_eq!(backoff_2, backoff_1 * 2);
        assert_eq!(backoff_3, backoff_2 * 2);
    }

    #[test]
    fn backoff_max_attempt_does_not_overflow() {
        let window_secs = RATE_LIMIT_WINDOW.as_secs();
        // MAX_RETRIES is the highest attempt number used in the backoff formula
        let max_backoff = window_secs.checked_mul(2u64.pow(MAX_RETRIES - 1));
        assert!(
            max_backoff.is_some(),
            "Backoff calculation overflows at MAX_RETRIES={}",
            MAX_RETRIES
        );
    }

    // ── BallDontLieClient construction ──────────────────────────────

    #[test]
    fn client_new_succeeds() {
        let client = BallDontLieClient::new("test-key".to_string());
        assert!(client.is_ok());
    }

    #[test]
    fn client_stores_api_key() {
        let client = BallDontLieClient::new("my-api-key".to_string()).unwrap();
        assert_eq!(client.api_key, "my-api-key");
    }
}
