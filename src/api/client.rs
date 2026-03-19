use anyhow::{Context, Result};
use chrono::NaiveDate;
use reqwest::Client;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::models::{ApiResponse, Game, Team};

const BASE_URL: &str = "https://api.balldontlie.io/v1";

/// Rate limit: free tier allows 5 requests per minute.
/// We send up to 5 requests concurrently, then wait for the remainder
/// of the 60-second window before the next batch.
const RATE_LIMIT_BATCH_SIZE: usize = 5;
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(61);

/// Maximum results per page (API max is 100).
const PER_PAGE: u32 = 100;

/// HTTP client for the balldontlie NBA API.
pub struct BallDontLieClient {
    client: Client,
    api_key: String,
}

impl BallDontLieClient {
    /// Create a new API client with the given API key.
    pub fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client, api_key })
    }

    /// Fetch all 30 NBA teams.
    pub async fn get_teams(&self) -> Result<Vec<Team>> {
        let url = format!("{BASE_URL}/teams");

        let resp = self
            .client
            .get(&url)
            .header("Authorization", &self.api_key)
            .send()
            .await
            .context("Failed to fetch teams")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Teams API returned {status}: {body}");
        }

        let api_resp: ApiResponse<Team> = resp
            .json()
            .await
            .context("Failed to parse teams response")?;

        debug!("Fetched {} teams", api_resp.data.len());
        Ok(api_resp.data)
    }

    /// Fetch all regular season games for a given season using parallel
    /// date-range fetching to minimize wall-clock time.
    ///
    /// Splits the season into date ranges and fetches them concurrently,
    /// staying within the 5 req/min rate limit by batching requests.
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
    /// the rate limit by processing in batches of 5 requests at a time.
    ///
    /// Each date range is paginated independently. We gather the first page
    /// of each range concurrently, then handle any remaining pages in
    /// subsequent batches.
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
                .map(|(i, s)| {
                    (
                        i,
                        s.start_date.clone(),
                        s.end_date.clone(),
                        s.cursor,
                    )
                })
                .collect();

            if pending.is_empty() {
                break;
            }

            // Process in batches of RATE_LIMIT_BATCH_SIZE
            for batch in pending.chunks(RATE_LIMIT_BATCH_SIZE) {
                let batch_start = std::time::Instant::now();
                let batch_size = batch.len();

                // Fire all requests in this batch concurrently
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

                // If there's more work to do, wait out the rate limit window
                let still_pending = states.iter().any(|s| !s.done);
                if still_pending {
                    let elapsed = batch_start.elapsed();
                    if elapsed < RATE_LIMIT_WINDOW {
                        let wait = RATE_LIMIT_WINDOW - elapsed;
                        info!(
                            "Rate limit: sent {batch_size} requests, waiting {:.0}s before next batch ({} games so far)",
                            wait.as_secs_f64(),
                            all_games.len()
                        );
                        tokio::time::sleep(wait).await;
                    }
                }
            }
        }

        info!(
            "Fetched {total_requests} pages for season {season}: {} total games",
            all_games.len()
        );
        Ok(all_games)
    }

    /// Fetch a single page of games for a date range.
    /// Returns the games and the next cursor (None if no more pages).
    async fn fetch_games_page(
        &self,
        season: u32,
        start_date: &str,
        end_date: &str,
        cursor: Option<u64>,
    ) -> Result<(Vec<Game>, Option<u64>)> {
        let mut request = self
            .client
            .get(format!("{BASE_URL}/games"))
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
            .with_context(|| {
                format!("Failed to fetch games ({start_date} to {end_date})")
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Games API returned {status} ({start_date} to {end_date}): {body}"
            );
        }

        let api_resp: ApiResponse<Game> = resp.json().await.with_context(|| {
            format!("Failed to parse games response ({start_date} to {end_date})")
        })?;

        let next_cursor = api_resp.meta.and_then(|m| m.next_cursor);
        let count = api_resp.data.len();
        debug!(
            "Fetched {count} games for {start_date}..{end_date} (cursor: {cursor:?}, next: {next_cursor:?})"
        );

        Ok((api_resp.data, next_cursor))
    }
}

/// Split an NBA season into 5 roughly equal date ranges for parallel fetching.
///
/// A typical NBA season runs from mid-October to mid-April (~180 game days).
/// We split this into 5 ranges so we can fetch the first page of each
/// concurrently within a single rate-limit window.
fn season_date_ranges(season: u32) -> Vec<(String, String)> {
    let start = NaiveDate::from_ymd_opt(season as i32, 10, 1)
        .expect("valid season start date");
    let end = NaiveDate::from_ymd_opt((season + 1) as i32, 7, 31)
        .expect("valid season end date");

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

/// Retry a request up to `max_retries` times with exponential backoff.
#[allow(dead_code)]
pub async fn retry_on_rate_limit<F, Fut, T>(max_retries: u32, mut f: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempt += 1;
                if attempt >= max_retries {
                    return Err(e);
                }
                let backoff = Duration::from_secs(2u64.pow(attempt));
                warn!(
                    "Request failed (attempt {attempt}/{max_retries}), retrying in {backoff:?}: {e}"
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
}
