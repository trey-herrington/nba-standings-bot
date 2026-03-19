use anyhow::{Context, Result};
use serenity::all::ChannelId;

/// Bot configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Discord bot token.
    pub discord_token: String,
    /// balldontlie API key.
    pub balldontlie_api_key: String,
    /// Discord channel ID for daily standings posts.
    pub channel_id: ChannelId,
    /// Cron schedule expression for daily posting.
    pub cron_schedule: String,
    /// Optional: override the NBA season year.
    pub nba_season: Option<u32>,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Result<Self> {
        let discord_token =
            std::env::var("DISCORD_TOKEN").context("DISCORD_TOKEN env var is required")?;

        let balldontlie_api_key = std::env::var("BALLDONTLIE_API_KEY")
            .context("BALLDONTLIE_API_KEY env var is required")?;

        let channel_id_raw =
            std::env::var("CHANNEL_ID").context("CHANNEL_ID env var is required")?;
        let channel_id = ChannelId::new(
            channel_id_raw
                .parse::<u64>()
                .context("CHANNEL_ID must be a valid u64")?,
        );

        let cron_schedule =
            std::env::var("CRON_SCHEDULE").unwrap_or_else(|_| "0 0 15 * * *".to_string());

        let nba_season = std::env::var("NBA_SEASON")
            .ok()
            .and_then(|s| s.parse::<u32>().ok());

        Ok(Self {
            discord_token,
            balldontlie_api_key,
            channel_id,
            cron_schedule,
            nba_season,
        })
    }
}
