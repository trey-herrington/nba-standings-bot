use std::sync::Arc;

use anyhow::{Context as _, Result};
use serenity::all::Http;
use serenity::builder::CreateMessage;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

use crate::config::Config;
use crate::standings::cache::StandingsCache;
use crate::standings::format::build_standings_embeds;

/// Start the daily standings scheduler.
///
/// Spawns a background task that refreshes the shared cache and posts
/// standings to the configured Discord channel on the configured cron schedule.
pub async fn start_scheduler(
    http: Arc<Http>,
    config: Config,
    cache: Arc<StandingsCache>,
) -> Result<JobScheduler> {
    let scheduler = JobScheduler::new()
        .await
        .context("Failed to create job scheduler")?;

    let cron_expr = config.cron_schedule.clone();
    info!("Scheduling daily standings post with cron: {cron_expr}");

    let job = Job::new_async(cron_expr.as_str(), move |_uuid, _lock| {
        let http = http.clone();
        let config = config.clone();
        let cache = cache.clone();

        Box::pin(async move {
            info!("Cron job triggered: refreshing cache and posting daily standings");
            if let Err(e) = post_standings_to_channel(&http, &config, &cache).await {
                error!("Failed to post daily standings: {e:#}");
            }
        })
    })
    .context("Failed to create cron job")?;

    scheduler
        .add(job)
        .await
        .context("Failed to add job to scheduler")?;

    scheduler
        .start()
        .await
        .context("Failed to start scheduler")?;

    info!("Scheduler started successfully");
    Ok(scheduler)
}

/// Build the Discord message containing standings embeds.
///
/// Extracted so it can be tested independently of the Discord HTTP layer.
fn build_standings_message(standings: &crate::standings::compute::Standings) -> CreateMessage {
    let embeds = build_standings_embeds(standings);
    CreateMessage::new().embeds(embeds)
}

/// Refresh the cache and post standings to the configured Discord channel.
async fn post_standings_to_channel(
    http: &Http,
    config: &Config,
    cache: &StandingsCache,
) -> Result<()> {
    // Force a refresh so the daily post always has the latest data
    let standings = cache.refresh().await?;
    let message = build_standings_message(&standings);

    config
        .channel_id
        .send_message(http, message)
        .await
        .context("Failed to send standings message to channel")?;

    let stats = cache.stats().await;
    info!(
        "Posted standings to channel {} (cache: {} games, latest: {:?})",
        config.channel_id, stats.game_count, stats.latest_game_date
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::Team;
    use crate::standings::compute::{Standings, TeamRecord};

    // ── Helpers ─────────────────────────────────────────────────────

    fn make_team(id: u64, name: &str, abbr: &str, conference: &str) -> Team {
        Team {
            id,
            conference: conference.to_string(),
            division: "Test".to_string(),
            city: "Test City".to_string(),
            name: name.to_string(),
            full_name: format!("Test City {name}"),
            abbreviation: abbr.to_string(),
        }
    }

    fn make_record(team: Team, wins: u32, losses: u32) -> TeamRecord {
        let total = wins + losses;
        let win_pct = if total > 0 {
            wins as f64 / total as f64
        } else {
            0.0
        };
        TeamRecord {
            team,
            wins,
            losses,
            win_pct,
        }
    }

    // ── build_standings_message ──────────────────────────────────────

    #[test]
    fn message_contains_both_conference_embeds() {
        let east = make_record(make_team(1, "Celtics", "BOS", "East"), 50, 20);
        let west = make_record(make_team(2, "Lakers", "LAL", "West"), 45, 25);

        let standings = Standings {
            eastern: vec![east],
            western: vec![west],
            season: 2025,
        };

        let message = build_standings_message(&standings);

        // Serialize the CreateMessage to JSON so we can inspect the embeds array.
        // CreateMessage derives Serialize, so this gives us the wire format.
        let json = serde_json::to_value(&message).expect("message should serialize");
        let embeds = json["embeds"].as_array().expect("embeds should be an array");

        assert_eq!(
            embeds.len(),
            2,
            "Scheduled message must contain exactly 2 embeds (Eastern + Western), got {}",
            embeds.len()
        );

        let titles: Vec<&str> = embeds
            .iter()
            .filter_map(|e| e["title"].as_str())
            .collect();

        assert!(
            titles.iter().any(|t| t.contains("Eastern")),
            "Missing Eastern Conference embed. Titles: {titles:?}"
        );
        assert!(
            titles.iter().any(|t| t.contains("Western")),
            "Missing Western Conference embed. Titles: {titles:?}"
        );
    }

    #[test]
    fn message_embeds_contain_team_data() {
        let east = make_record(make_team(1, "Celtics", "BOS", "East"), 50, 20);
        let west = make_record(make_team(2, "Lakers", "LAL", "West"), 45, 25);

        let standings = Standings {
            eastern: vec![east],
            western: vec![west],
            season: 2025,
        };

        let message = build_standings_message(&standings);
        let json = serde_json::to_value(&message).expect("message should serialize");
        let embeds = json["embeds"].as_array().unwrap();

        let descriptions: Vec<&str> = embeds
            .iter()
            .filter_map(|e| e["description"].as_str())
            .collect();

        assert!(
            descriptions.iter().any(|d| d.contains("BOS")),
            "Eastern embed should contain BOS team data"
        );
        assert!(
            descriptions.iter().any(|d| d.contains("LAL")),
            "Western embed should contain LAL team data"
        );
    }

    #[test]
    fn message_with_empty_standings_still_has_both_embeds() {
        let standings = Standings {
            eastern: vec![],
            western: vec![],
            season: 2025,
        };

        let message = build_standings_message(&standings);
        let json = serde_json::to_value(&message).expect("message should serialize");
        let embeds = json["embeds"].as_array().expect("embeds should be an array");

        assert_eq!(
            embeds.len(),
            2,
            "Even with no teams, message must have both conference embeds"
        );
    }

    /// Regression test for the bug where using CreateMessage::embed() in a
    /// loop caused the Eastern Conference embed to be replaced by the Western
    /// one, since serenity's embed() replaces all existing embeds rather
    /// than appending. The fix uses embeds() (plural) to set both at once.
    #[test]
    fn embed_replace_regression() {
        let east = make_record(make_team(1, "Celtics", "BOS", "East"), 50, 20);
        let west = make_record(make_team(2, "Lakers", "LAL", "West"), 45, 25);

        let standings = Standings {
            eastern: vec![east],
            western: vec![west],
            season: 2025,
        };

        let embeds = build_standings_embeds(&standings);
        assert_eq!(embeds.len(), 2, "build_standings_embeds should return 2 embeds");

        // Demonstrate the bug: the old code used .embed() in a loop, which
        // replaces rather than appends in serenity's CreateMessage.
        let mut buggy_message = CreateMessage::new();
        for embed in build_standings_embeds(&standings) {
            buggy_message = buggy_message.embed(embed);
        }
        let buggy_json = serde_json::to_value(&buggy_message).unwrap();
        let buggy_embeds = buggy_json["embeds"].as_array().unwrap();
        assert_eq!(
            buggy_embeds.len(),
            1,
            "The old .embed() loop should produce only 1 embed (the bug)"
        );
        assert!(
            buggy_embeds[0]["title"].as_str().unwrap().contains("Western"),
            "The old .embed() loop should only keep the last (Western) embed"
        );

        // Verify the fix: build_standings_message uses .embeds() (plural)
        let fixed_message = build_standings_message(&standings);
        let fixed_json = serde_json::to_value(&fixed_message).unwrap();
        let fixed_embeds = fixed_json["embeds"].as_array().unwrap();
        assert_eq!(
            fixed_embeds.len(),
            2,
            "The fixed .embeds() call must preserve both conference embeds"
        );
        assert!(
            fixed_embeds[0]["title"].as_str().unwrap().contains("Eastern"),
            "First embed should be Eastern Conference"
        );
        assert!(
            fixed_embeds[1]["title"].as_str().unwrap().contains("Western"),
            "Second embed should be Western Conference"
        );
    }
}
