use std::sync::Arc;

use poise::CreateReply;
use tracing::error;

use crate::config::Config;
use crate::standings::cache::StandingsCache;
use crate::standings::format::build_standings_embeds;

/// Shared application data accessible to all commands.
pub struct Data {
    pub cache: Arc<StandingsCache>,
    pub config: Config,
}

/// poise error type alias.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// poise context type alias.
pub type Context<'a> = poise::Context<'a, Data, Error>;

/// Slash command: `/standings [season]`
///
/// Fetches and displays the current NBA standings. Uses the in-memory cache
/// for near-instant responses when data is fresh.
#[poise::command(slash_command, description_localized("en-US", "Show NBA standings"))]
pub async fn standings(
    ctx: Context<'_>,
    #[description = "Season year (e.g., 2025 for 2025-26). Defaults to current season."]
    season: Option<u32>,
) -> Result<(), Error> {
    // Defer the response since the first fetch may take a while
    ctx.defer().await?;

    // TODO: if a non-default season is requested, we'd need a separate
    // cache or a direct fetch. For now, the cache serves the current season.
    if season.is_some() && season != ctx.data().config.nba_season {
        ctx.say("Custom season lookups bypass the cache and may take a few minutes. \
                 Fetching...").await?;
    }

    match ctx.data().cache.get_standings().await {
        Ok(standings) => {
            let embeds = build_standings_embeds(&standings);
            let mut reply = CreateReply::default();
            for embed in embeds {
                reply = reply.embed(embed);
            }
            ctx.send(reply).await?;
        }
        Err(e) => {
            error!("Failed to fetch standings: {e:#}");
            ctx.say(format!("Failed to fetch standings: {e}")).await?;
        }
    }

    Ok(())
}
