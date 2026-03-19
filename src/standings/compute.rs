use std::collections::HashMap;

use chrono::{Datelike, Utc};

use crate::api::models::{Game, Team};

/// A team's win-loss record for a season.
#[derive(Debug, Clone)]
pub struct TeamRecord {
    pub team: Team,
    pub wins: u32,
    pub losses: u32,
    pub win_pct: f64,
}

/// Standings split by conference.
#[derive(Debug, Clone)]
pub struct Standings {
    pub eastern: Vec<TeamRecord>,
    pub western: Vec<TeamRecord>,
    pub season: u32,
}

/// Determine the current NBA season year.
///
/// The NBA season starts in October. If we're in Oct-Dec, the season year
/// equals the current year. If we're in Jan-Sep, it equals the previous year.
/// For example, in March 2026 the current season is 2025 (the 2025-26 season).
pub fn current_nba_season() -> u32 {
    let now = Utc::now();
    let year = now.year() as u32;
    let month = now.month();

    if month >= 10 {
        year
    } else {
        year - 1
    }
}

/// Compute standings from a list of teams and games.
///
/// Only games with status "Final" are counted. Games are tallied per team
/// and results are split into Eastern and Western conferences, sorted by
/// win percentage descending.
pub fn compute_standings(teams: &[Team], games: &[Game], season: u32) -> Standings {
    // Initialize records for all teams
    let mut records: HashMap<u64, TeamRecord> = teams
        .iter()
        .map(|team| {
            (
                team.id,
                TeamRecord {
                    team: team.clone(),
                    wins: 0,
                    losses: 0,
                    win_pct: 0.0,
                },
            )
        })
        .collect();

    // Tally wins and losses from completed games
    for game in games {
        if game.status != "Final" {
            continue;
        }

        let (home_score, visitor_score) = match (game.home_team_score, game.visitor_team_score) {
            (Some(h), Some(v)) => (h, v),
            _ => continue,
        };

        if home_score > visitor_score {
            // Home team won
            if let Some(record) = records.get_mut(&game.home_team.id) {
                record.wins += 1;
            }
            if let Some(record) = records.get_mut(&game.visitor_team.id) {
                record.losses += 1;
            }
        } else if visitor_score > home_score {
            // Visitor team won
            if let Some(record) = records.get_mut(&game.visitor_team.id) {
                record.wins += 1;
            }
            if let Some(record) = records.get_mut(&game.home_team.id) {
                record.losses += 1;
            }
        }
        // Ties don't happen in the NBA, but if scores are equal we skip
    }

    // Calculate win percentages
    for record in records.values_mut() {
        let total = record.wins + record.losses;
        record.win_pct = if total > 0 {
            record.wins as f64 / total as f64
        } else {
            0.0
        };
    }

    // Split into conferences and sort
    let mut eastern: Vec<TeamRecord> = records
        .values()
        .filter(|r| r.team.conference == "East")
        .cloned()
        .collect();

    let mut western: Vec<TeamRecord> = records
        .values()
        .filter(|r| r.team.conference == "West")
        .cloned()
        .collect();

    // Sort by win percentage descending, then by wins descending as tiebreaker
    let sort_fn = |a: &TeamRecord, b: &TeamRecord| {
        b.win_pct
            .partial_cmp(&a.win_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.wins.cmp(&a.wins))
    };

    eastern.sort_by(sort_fn);
    western.sort_by(sort_fn);

    Standings {
        eastern,
        western,
        season,
    }
}
