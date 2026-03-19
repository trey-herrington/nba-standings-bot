# nba-standings-bot

A Discord bot that posts NBA standings daily and supports on-demand `/standings` queries.

## Features

- **Daily auto-post**: Posts Eastern and Western conference standings to a configured channel on a cron schedule (default: 10:00 AM ET)
- **Slash command**: `/standings [season]` for on-demand standings checks
- **Rich embeds**: Color-coded conference tables with W-L records and win percentages
- **Free tier**: Computes standings from game data using the free balldontlie API (no paid tier required)

## Prerequisites

- [Rust](https://rustup.rs/) (1.85+ recommended, edition 2021)
- A Discord bot token ([Discord Developer Portal](https://discord.com/developers/applications))
- A balldontlie API key ([app.balldontlie.io](https://app.balldontlie.io) -- free)

## Setup

### 1. Create a Discord Bot

1. Go to the [Discord Developer Portal](https://discord.com/developers/applications)
2. Click "New Application" and give it a name
3. Go to the "Bot" tab and click "Reset Token" to get your bot token
4. Under "Privileged Gateway Intents", no special intents are needed
5. Go to "OAuth2" > "URL Generator", select the `bot` and `applications.commands` scopes
6. Under "Bot Permissions", select "Send Messages" and "Embed Links"
7. Use the generated URL to invite the bot to your server

### 2. Get a balldontlie API Key

1. Sign up at [app.balldontlie.io](https://app.balldontlie.io) (free)
2. Copy your API key from the dashboard

### 3. Configure Environment

```sh
cp .env.example .env
```

Edit `.env` with your values:

```
DISCORD_TOKEN=your_discord_bot_token
BALLDONTLIE_API_KEY=your_api_key
CHANNEL_ID=123456789012345678
CRON_SCHEDULE=0 0 15 * * *
```

To get the channel ID, enable Developer Mode in Discord (User Settings > Advanced > Developer Mode), then right-click the target channel and select "Copy Channel ID".

### 4. Build and Run

```sh
cargo run
```

For release builds:

```sh
cargo build --release
./target/release/nba-standings-bot
```

## Configuration

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DISCORD_TOKEN` | Yes | -- | Discord bot token |
| `BALLDONTLIE_API_KEY` | Yes | -- | balldontlie.io API key |
| `CHANNEL_ID` | Yes | -- | Channel ID for daily standings posts |
| `CRON_SCHEDULE` | No | `0 0 15 * * *` | Cron expression (sec min hr day mon dow) |
| `NBA_SEASON` | No | auto-detect | Override season year (e.g., `2025`) |
| `RUST_LOG` | No | `info` | Log level filter |

## Usage

### Daily Auto-Post

The bot automatically posts standings to the configured channel at the scheduled time. The default schedule is 10:00 AM Eastern (3:00 PM UTC).

### Slash Command

Use `/standings` in any channel the bot has access to. Optionally specify a season year:

```
/standings
/standings season:2024
```

## How It Works

The bot uses the free balldontlie API endpoints (Teams + Games) to compute standings rather than the paid Team Standings endpoint. It fetches all regular season games, tallies wins and losses per team, and sorts by win percentage within each conference.

**Note**: The free API tier has a rate limit of 5 requests/minute. Fetching a full season of games (~13 pages) takes approximately 3 minutes due to rate limiting delays.

## License

See [LICENSE](LICENSE) for details.
