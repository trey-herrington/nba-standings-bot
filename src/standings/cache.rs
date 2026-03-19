use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::NaiveDate;
use tokio::sync::RwLock;
use tracing::{debug, info};

use crate::api::client::BallDontLieClient;
use crate::api::models::{Game, Team};
use crate::standings::compute::{compute_standings, current_nba_season, Standings};

/// How long cached standings remain fresh before an incremental refresh
/// is triggered on the next request. Default: 1 hour.
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// Inner cache state, protected by a RwLock.
struct CacheInner {
    /// Cached teams (rarely changes, fetched once).
    teams: Vec<Team>,
    /// All cached games for the current season, keyed by game ID to avoid duplicates.
    games: HashMap<u64, Game>,
    /// The most recent game date we've seen (YYYY-MM-DD), used for incremental fetches.
    latest_game_date: Option<String>,
    /// The season these games belong to.
    season: u32,
    /// Pre-computed standings from the cached data.
    standings: Option<Standings>,
    /// When the cache was last refreshed.
    last_refresh: Option<std::time::Instant>,
}

/// Thread-safe, async-friendly cache for NBA standings data.
///
/// Stores teams and games in memory, supports incremental updates
/// (only fetching games newer than what's already cached), and
/// pre-computes standings so `/standings` responses are near-instant.
pub struct StandingsCache {
    inner: RwLock<CacheInner>,
    api_client: Arc<BallDontLieClient>,
    season_override: Option<u32>,
}

impl StandingsCache {
    /// Create a new empty cache.
    pub fn new(api_client: Arc<BallDontLieClient>, season_override: Option<u32>) -> Self {
        let season = season_override.unwrap_or_else(current_nba_season);

        Self {
            inner: RwLock::new(CacheInner {
                teams: Vec::new(),
                games: HashMap::new(),
                latest_game_date: None,
                season,
                standings: None,
                last_refresh: None,
            }),
            api_client,
            season_override,
        }
    }

    /// Get standings, refreshing the cache incrementally if stale.
    ///
    /// - If the cache has never been populated, does a full fetch.
    /// - If the cache is older than the TTL, does an incremental fetch
    ///   (only games since the last known date).
    /// - If the cache is fresh, returns the pre-computed standings instantly.
    pub async fn get_standings(&self) -> Result<Standings> {
        // Fast path: check if cache is fresh under a read lock
        {
            let inner = self.inner.read().await;
            if let (Some(standings), Some(last_refresh)) =
                (&inner.standings, inner.last_refresh)
            {
                if last_refresh.elapsed() < CACHE_TTL {
                    debug!(
                        "Cache hit: standings are {:.0}s old (TTL: {}s)",
                        last_refresh.elapsed().as_secs_f64(),
                        CACHE_TTL.as_secs()
                    );
                    return Ok(standings.clone());
                }
            }
        }

        // Cache is stale or empty -- refresh
        self.refresh().await
    }

    /// Force a full or incremental refresh of the cache.
    ///
    /// Returns the newly computed standings. This is called by the daily
    /// scheduler and when the cache TTL expires.
    pub async fn refresh(&self) -> Result<Standings> {
        let season = self.season_override.unwrap_or_else(current_nba_season);

        // Determine if this is a full or incremental fetch
        let (needs_teams, start_date, old_season) = {
            let inner = self.inner.read().await;
            let needs_teams = inner.teams.is_empty();
            let start_date = inner.latest_game_date.clone();
            (needs_teams, start_date, inner.season)
        };

        // If the season changed (e.g., October rollover), do a full fetch
        let season_changed = season != old_season;

        // Fetch teams if we don't have them or season changed
        let teams = if needs_teams || season_changed {
            info!("Fetching teams from API");
            self.api_client.get_teams().await?
        } else {
            self.inner.read().await.teams.clone()
        };

        // Fetch games: incremental if we have a latest date and same season
        let new_games = if let (Some(ref date), false) = (&start_date, season_changed) {
            info!("Incremental refresh: fetching games since {date}");
            self.api_client.get_games_since(season, Some(date)).await?
        } else {
            info!("Full refresh: fetching all games for season {season}");
            self.api_client.get_season_games(season).await?
        };

        // Update the cache under a write lock
        let standings = {
            let mut inner = self.inner.write().await;

            // Reset if season changed
            if season_changed {
                inner.games.clear();
                inner.latest_game_date = None;
                inner.season = season;
            }

            inner.teams = teams;

            // Merge new games (upsert by game ID to handle score updates
            // for games that were in-progress during the previous fetch)
            let mut latest_date: Option<NaiveDate> = inner
                .latest_game_date
                .as_ref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());

            for game in new_games {
                if let Ok(game_date) = NaiveDate::parse_from_str(&game.date, "%Y-%m-%d") {
                    match &latest_date {
                        Some(current) if game_date > *current => {
                            latest_date = Some(game_date);
                        }
                        None => {
                            latest_date = Some(game_date);
                        }
                        _ => {}
                    }
                }
                inner.games.insert(game.id, game);
            }

            inner.latest_game_date = latest_date.map(|d| d.format("%Y-%m-%d").to_string());
            inner.last_refresh = Some(std::time::Instant::now());

            // Recompute standings from all cached games
            let all_games: Vec<Game> = inner.games.values().cloned().collect();
            let standings = compute_standings(&inner.teams, &all_games, inner.season);

            info!(
                "Cache refreshed: {} teams, {} games, latest date: {:?}",
                inner.teams.len(),
                inner.games.len(),
                inner.latest_game_date
            );

            inner.standings = Some(standings.clone());
            standings
        };

        Ok(standings)
    }

    /// Get cache stats for logging/debugging.
    pub async fn stats(&self) -> CacheStats {
        let inner = self.inner.read().await;
        CacheStats {
            team_count: inner.teams.len(),
            game_count: inner.games.len(),
            season: inner.season,
            latest_game_date: inner.latest_game_date.clone(),
            age_secs: inner.last_refresh.map(|t| t.elapsed().as_secs()),
        }
    }
}

/// Summary of cache state for logging.
#[derive(Debug)]
#[allow(dead_code)]
pub struct CacheStats {
    pub team_count: usize,
    pub game_count: usize,
    pub season: u32,
    pub latest_game_date: Option<String>,
    pub age_secs: Option<u64>,
}
