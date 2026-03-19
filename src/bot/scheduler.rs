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

/// Refresh the cache and post standings to the configured Discord channel.
async fn post_standings_to_channel(
    http: &Http,
    config: &Config,
    cache: &StandingsCache,
) -> Result<()> {
    // Force a refresh so the daily post always has the latest data
    let standings = cache.refresh().await?;
    let embeds = build_standings_embeds(&standings);

    let mut message = CreateMessage::new();
    for embed in embeds {
        message = message.embed(embed);
    }

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
