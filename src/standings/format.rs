use chrono::Utc;
use serenity::all::Colour;
use serenity::builder::CreateEmbed;

use super::compute::{Standings, TeamRecord};

/// Format the season display string (e.g., "2025-26").
fn season_display(season: u32) -> String {
    let next = (season + 1) % 100;
    format!("{season}-{next:02}")
}

/// Build a formatted table string from a list of team records.
fn build_table(records: &[TeamRecord]) -> String {
    let mut lines = Vec::with_capacity(records.len() + 2);

    // Header
    lines.push(format!(
        "{:<4} {:<4} {:>3} {:>3}  {:>5}",
        "#", "Team", "W", "L", "PCT"
    ));
    lines.push("─".repeat(24));

    // Team rows
    for (i, record) in records.iter().enumerate() {
        lines.push(format!(
            "{:<4} {:<4} {:>3} {:>3}  {:.3}",
            i + 1,
            record.team.abbreviation,
            record.wins,
            record.losses,
            record.win_pct
        ));
    }

    lines.join("\n")
}

/// Build Discord embeds for the standings (one for each conference).
pub fn build_standings_embeds(standings: &Standings) -> Vec<CreateEmbed> {
    let season_str = season_display(standings.season);
    let timestamp = Utc::now().format("%B %-d, %Y at %-I:%M %p UTC").to_string();

    let eastern_table = build_table(&standings.eastern);
    let western_table = build_table(&standings.western);

    let eastern_embed = CreateEmbed::new()
        .title(format!("Eastern Conference — {season_str}"))
        .description(format!("```\n{eastern_table}\n```"))
        .colour(Colour::from_rgb(29, 66, 138))
        .footer(serenity::builder::CreateEmbedFooter::new(format!(
            "Updated {timestamp}"
        )));

    let western_embed = CreateEmbed::new()
        .title(format!("Western Conference — {season_str}"))
        .description(format!("```\n{western_table}\n```"))
        .colour(Colour::from_rgb(200, 16, 46))
        .footer(serenity::builder::CreateEmbedFooter::new(format!(
            "Updated {timestamp}"
        )));

    vec![eastern_embed, western_embed]
}
