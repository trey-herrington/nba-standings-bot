mod api;
mod bot;
mod config;
mod standings;

use std::sync::Arc;

use anyhow::Result;
use poise::serenity_prelude as serenity;
use tracing::{error, info};

use api::client::BallDontLieClient;
use bot::commands::{self, Data};
use bot::scheduler;
use config::Config;
use standings::cache::StandingsCache;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file (ignore errors if it doesn't exist)
    let _ = dotenvy::dotenv();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Starting NBA Standings Bot");

    // Load configuration
    let config = Config::from_env()?;
    info!(
        "Configuration loaded: channel={}, cron={}",
        config.channel_id, config.cron_schedule
    );

    // Save the token before config is moved into closures
    let discord_token = config.discord_token.clone();

    // Build the shared API client and standings cache
    let api_client = Arc::new(BallDontLieClient::new(config.balldontlie_api_key.clone())?);
    let cache = Arc::new(StandingsCache::new(api_client, config.nba_season));

    // Pre-warm the cache in the background so the first /standings is instant.
    // This runs concurrently with the Discord connection handshake.
    let warmup_cache = cache.clone();
    let _warmup_handle = tokio::spawn(async move {
        info!("Pre-warming standings cache...");
        match warmup_cache.refresh().await {
            Ok(standings) => {
                info!(
                    "Cache pre-warmed: {} Eastern, {} Western teams",
                    standings.eastern.len(),
                    standings.western.len()
                );
            }
            Err(e) => {
                error!("Cache pre-warm failed (will retry on first request): {e:#}");
            }
        }
    });

    // Clone references for the setup closure
    let scheduler_config = config.clone();
    let scheduler_cache = cache.clone();

    // Set up poise framework
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![commands::standings()],
            on_error: |error| {
                Box::pin(async move {
                    match error {
                        poise::FrameworkError::Command { error, ctx, .. } => {
                            error!("Command error: {error:#}");
                            let _ = ctx.say(format!("An error occurred: {error}")).await;
                        }
                        other => {
                            if let Err(e) = poise::builtins::on_error(other).await {
                                error!("Error handling framework error: {e:#}");
                            }
                        }
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                info!("Bot connected as {}", ready.user.name);

                // Register slash commands globally
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                info!("Slash commands registered globally");

                // Start the daily scheduler with the shared cache
                let http = ctx.http.clone();
                match scheduler::start_scheduler(http, scheduler_config, scheduler_cache).await {
                    Ok(_scheduler) => {
                        info!("Daily scheduler started");
                        // Leak the scheduler so it lives for the program's lifetime.
                        // This is intentional -- the scheduler must not be dropped.
                        std::mem::forget(_scheduler);
                    }
                    Err(e) => {
                        error!("Failed to start scheduler: {e:#}");
                        // Continue running -- the bot still works for slash commands
                    }
                }

                Ok(Data { cache, config })
            })
        })
        .build();

    // Build the serenity client with minimal intents
    let intents = serenity::GatewayIntents::empty();
    let mut client = serenity::ClientBuilder::new(&discord_token, intents)
        .framework(framework)
        .await?;

    info!("Starting Discord client...");
    client.start().await?;

    Ok(())
}
