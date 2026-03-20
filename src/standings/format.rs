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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::Team;
    use crate::standings::compute::Standings;

    // ── Helpers ─────────────────────────────────────────────────────

    fn make_team(id: u64, name: &str, abbreviation: &str, conference: &str) -> Team {
        Team {
            id,
            conference: conference.to_string(),
            division: "Test".to_string(),
            city: "Test City".to_string(),
            name: name.to_string(),
            full_name: format!("Test City {name}"),
            abbreviation: abbreviation.to_string(),
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

    // ── season_display ──────────────────────────────────────────────

    #[test]
    fn season_display_normal() {
        assert_eq!(season_display(2025), "2025-26");
    }

    #[test]
    fn season_display_century_boundary() {
        assert_eq!(season_display(2099), "2099-00");
    }

    #[test]
    fn season_display_single_digit_next_year() {
        assert_eq!(season_display(2008), "2008-09");
    }

    #[test]
    fn season_display_double_digit_next_year() {
        assert_eq!(season_display(2019), "2019-20");
    }

    // ── build_table ─────────────────────────────────────────────────

    #[test]
    fn build_table_empty_records() {
        let table = build_table(&[]);
        let lines: Vec<&str> = table.lines().collect();
        // Should have header and separator, no data rows
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn build_table_contains_header() {
        let table = build_table(&[]);
        assert!(table.contains("#"));
        assert!(table.contains("Team"));
        assert!(table.contains("W"));
        assert!(table.contains("L"));
        assert!(table.contains("PCT"));
    }

    #[test]
    fn build_table_contains_separator() {
        let table = build_table(&[]);
        assert!(table.contains("─"));
    }

    #[test]
    fn build_table_single_team() {
        let team = make_team(1, "Celtics", "BOS", "East");
        let record = make_record(team, 50, 20);
        let table = build_table(&[record]);
        let lines: Vec<&str> = table.lines().collect();

        // header + separator + 1 data row
        assert_eq!(lines.len(), 3);
        assert!(lines[2].contains("BOS"));
        assert!(lines[2].contains("50"));
        assert!(lines[2].contains("20"));
    }

    #[test]
    fn build_table_ranking_numbers() {
        let team_a = make_record(make_team(1, "A", "AAA", "East"), 50, 10);
        let team_b = make_record(make_team(2, "B", "BBB", "East"), 40, 20);
        let team_c = make_record(make_team(3, "C", "CCC", "East"), 30, 30);

        let table = build_table(&[team_a, team_b, team_c]);
        let lines: Vec<&str> = table.lines().collect();

        // Data rows start at index 2
        assert!(lines[2].starts_with("1"));
        assert!(lines[3].starts_with("2"));
        assert!(lines[4].starts_with("3"));
    }

    #[test]
    fn build_table_win_pct_format() {
        let team = make_team(1, "Celtics", "BOS", "East");
        let record = make_record(team, 2, 1); // .667
        let table = build_table(&[record]);

        assert!(
            table.contains("0.667"),
            "Expected .667 format, got:\n{}",
            table
        );
    }

    #[test]
    fn build_table_perfect_record_format() {
        let team = make_team(1, "Celtics", "BOS", "East");
        let record = make_record(team, 10, 0); // 1.000
        let table = build_table(&[record]);

        assert!(
            table.contains("1.000"),
            "Expected 1.000 format, got:\n{}",
            table
        );
    }

    #[test]
    fn build_table_zero_record_format() {
        let team = make_team(1, "Celtics", "BOS", "East");
        let record = make_record(team, 0, 0); // 0.000
        let table = build_table(&[record]);

        assert!(
            table.contains("0.000"),
            "Expected 0.000 format, got:\n{}",
            table
        );
    }

    // ── build_standings_embeds ───────────────────────────────────────

    #[test]
    fn build_standings_embeds_returns_two_embeds() {
        let standings = Standings {
            eastern: vec![],
            western: vec![],
            season: 2025,
        };
        let embeds = build_standings_embeds(&standings);
        assert_eq!(embeds.len(), 2);
    }

    #[test]
    fn build_standings_embeds_with_team_data() {
        let east_record = make_record(make_team(1, "Celtics", "BOS", "East"), 50, 20);
        let west_record = make_record(make_team(2, "Lakers", "LAL", "West"), 45, 25);

        let standings = Standings {
            eastern: vec![east_record],
            western: vec![west_record],
            season: 2025,
        };

        let embeds = build_standings_embeds(&standings);
        assert_eq!(embeds.len(), 2);
    }
}
