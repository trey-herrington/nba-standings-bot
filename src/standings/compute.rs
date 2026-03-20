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

#[cfg(test)]
mod tests {
    use super::*;

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

    fn make_game(
        id: u64,
        home: &Team,
        visitor: &Team,
        home_score: Option<u32>,
        visitor_score: Option<u32>,
        status: &str,
        date: &str,
    ) -> Game {
        Game {
            id,
            date: date.to_string(),
            season: 2025,
            status: status.to_string(),
            period: Some(4),
            postseason: false,
            postponed: None,
            home_team_score: home_score,
            visitor_team_score: visitor_score,
            home_team: home.clone(),
            visitor_team: visitor.clone(),
        }
    }

    fn east_team(id: u64, name: &str, abbr: &str) -> Team {
        make_team(id, name, abbr, "East")
    }

    fn west_team(id: u64, name: &str, abbr: &str) -> Team {
        make_team(id, name, abbr, "West")
    }

    // ── current_nba_season ──────────────────────────────────────────

    #[test]
    fn current_nba_season_returns_a_plausible_value() {
        let season = current_nba_season();
        // In 2026, the season should be 2025 (Jan-Sep) or 2026 (Oct-Dec)
        assert!(
            season >= 2020 && season <= 2040,
            "Season {season} seems implausible"
        );
    }

    // ── compute_standings: basic tallying ────────────────────────────

    #[test]
    fn empty_games_produces_zero_records() {
        let celtics = east_team(1, "Celtics", "BOS");
        let lakers = west_team(2, "Lakers", "LAL");

        let standings = compute_standings(&[celtics, lakers], &[], 2025);

        assert_eq!(standings.eastern.len(), 1);
        assert_eq!(standings.western.len(), 1);
        assert_eq!(standings.eastern[0].wins, 0);
        assert_eq!(standings.eastern[0].losses, 0);
        assert_eq!(standings.eastern[0].win_pct, 0.0);
        assert_eq!(standings.western[0].wins, 0);
        assert_eq!(standings.western[0].losses, 0);
    }

    #[test]
    fn home_win_tallied_correctly() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let game = make_game(
            100,
            &celtics,
            &knicks,
            Some(110),
            Some(95),
            "Final",
            "2025-11-01",
        );
        let standings = compute_standings(&[celtics, knicks], &[game], 2025);

        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        let nyk = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "NYK")
            .unwrap();

        assert_eq!(bos.wins, 1);
        assert_eq!(bos.losses, 0);
        assert_eq!(nyk.wins, 0);
        assert_eq!(nyk.losses, 1);
    }

    #[test]
    fn visitor_win_tallied_correctly() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let game = make_game(
            100,
            &celtics,
            &knicks,
            Some(90),
            Some(105),
            "Final",
            "2025-11-01",
        );
        let standings = compute_standings(&[celtics, knicks], &[game], 2025);

        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        let nyk = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "NYK")
            .unwrap();

        assert_eq!(bos.wins, 0);
        assert_eq!(bos.losses, 1);
        assert_eq!(nyk.wins, 1);
        assert_eq!(nyk.losses, 0);
    }

    #[test]
    fn multiple_games_accumulate() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let games = vec![
            make_game(
                100,
                &celtics,
                &knicks,
                Some(110),
                Some(95),
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &celtics,
                &knicks,
                Some(100),
                Some(105),
                "Final",
                "2025-11-05",
            ),
            make_game(
                102,
                &knicks,
                &celtics,
                Some(90),
                Some(110),
                "Final",
                "2025-11-10",
            ),
        ];

        let standings = compute_standings(&[celtics, knicks], &games, 2025);
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        let nyk = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "NYK")
            .unwrap();

        // BOS: won game 100 (home), lost game 101 (home), won game 102 (visitor) = 2-1
        assert_eq!(bos.wins, 2);
        assert_eq!(bos.losses, 1);
        // NYK: lost game 100 (visitor), won game 101 (visitor), lost game 102 (home) = 1-2
        assert_eq!(nyk.wins, 1);
        assert_eq!(nyk.losses, 2);
    }

    // ── compute_standings: filtering ────────────────────────────────

    #[test]
    fn non_final_games_are_ignored() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let games = vec![
            make_game(
                100,
                &celtics,
                &knicks,
                Some(50),
                Some(45),
                "3rd Qtr",
                "2025-11-01",
            ),
            make_game(
                101,
                &celtics,
                &knicks,
                None,
                None,
                "Scheduled",
                "2025-11-05",
            ),
        ];

        let standings = compute_standings(&[celtics, knicks], &games, 2025);
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();

        assert_eq!(bos.wins, 0);
        assert_eq!(bos.losses, 0);
    }

    #[test]
    fn games_with_missing_scores_are_ignored() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let games = vec![
            make_game(
                100,
                &celtics,
                &knicks,
                Some(110),
                None,
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &celtics,
                &knicks,
                None,
                Some(105),
                "Final",
                "2025-11-05",
            ),
            make_game(102, &celtics, &knicks, None, None, "Final", "2025-11-10"),
        ];

        let standings = compute_standings(&[celtics, knicks], &games, 2025);
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();

        assert_eq!(bos.wins, 0);
        assert_eq!(bos.losses, 0);
    }

    #[test]
    fn tied_scores_are_ignored() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let game = make_game(
            100,
            &celtics,
            &knicks,
            Some(100),
            Some(100),
            "Final",
            "2025-11-01",
        );
        let standings = compute_standings(&[celtics, knicks], &[game], 2025);

        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        let nyk = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "NYK")
            .unwrap();

        assert_eq!(bos.wins, 0);
        assert_eq!(bos.losses, 0);
        assert_eq!(nyk.wins, 0);
        assert_eq!(nyk.losses, 0);
    }

    #[test]
    fn games_for_unknown_teams_are_silently_skipped() {
        let celtics = east_team(1, "Celtics", "BOS");
        let unknown = east_team(999, "Unknowns", "UNK");

        // Game involves team 999 which is not in our teams list
        let game = make_game(
            100,
            &celtics,
            &unknown,
            Some(110),
            Some(95),
            "Final",
            "2025-11-01",
        );
        let standings = compute_standings(&[celtics], &[game], 2025);

        // BOS gets the win (home team won), UNK loss is silently skipped
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        assert_eq!(bos.wins, 1);
        assert_eq!(bos.losses, 0);
    }

    // ── compute_standings: win percentage ────────────────────────────

    #[test]
    fn win_percentage_calculated_correctly() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let games = vec![
            make_game(
                100,
                &celtics,
                &knicks,
                Some(110),
                Some(95),
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &celtics,
                &knicks,
                Some(105),
                Some(100),
                "Final",
                "2025-11-05",
            ),
            make_game(
                102,
                &knicks,
                &celtics,
                Some(120),
                Some(90),
                "Final",
                "2025-11-10",
            ),
        ];

        let standings = compute_standings(&[celtics, knicks], &games, 2025);
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();

        // BOS: 2 wins, 1 loss = 0.667
        assert!((bos.win_pct - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn zero_games_produces_zero_win_pct() {
        let celtics = east_team(1, "Celtics", "BOS");
        let standings = compute_standings(&[celtics], &[], 2025);
        assert_eq!(standings.eastern[0].win_pct, 0.0);
    }

    #[test]
    fn perfect_record_is_one_point_zero() {
        let celtics = east_team(1, "Celtics", "BOS");
        let knicks = east_team(2, "Knicks", "NYK");

        let games = vec![
            make_game(
                100,
                &celtics,
                &knicks,
                Some(110),
                Some(95),
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &celtics,
                &knicks,
                Some(105),
                Some(100),
                "Final",
                "2025-11-05",
            ),
        ];

        let standings = compute_standings(&[celtics, knicks], &games, 2025);
        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        assert!((bos.win_pct - 1.0).abs() < f64::EPSILON);
    }

    // ── compute_standings: conference splitting ─────────────────────

    #[test]
    fn teams_split_into_correct_conferences() {
        let celtics = east_team(1, "Celtics", "BOS");
        let heat = east_team(2, "Heat", "MIA");
        let lakers = west_team(3, "Lakers", "LAL");
        let warriors = west_team(4, "Warriors", "GSW");

        let standings = compute_standings(&[celtics, heat, lakers, warriors], &[], 2025);

        assert_eq!(standings.eastern.len(), 2);
        assert_eq!(standings.western.len(), 2);

        let east_abbrs: Vec<&str> = standings
            .eastern
            .iter()
            .map(|r| r.team.abbreviation.as_str())
            .collect();
        let west_abbrs: Vec<&str> = standings
            .western
            .iter()
            .map(|r| r.team.abbreviation.as_str())
            .collect();

        assert!(east_abbrs.contains(&"BOS"));
        assert!(east_abbrs.contains(&"MIA"));
        assert!(west_abbrs.contains(&"LAL"));
        assert!(west_abbrs.contains(&"GSW"));
    }

    #[test]
    fn cross_conference_game_tallied_for_both_teams() {
        let celtics = east_team(1, "Celtics", "BOS");
        let lakers = west_team(2, "Lakers", "LAL");

        let game = make_game(
            100,
            &celtics,
            &lakers,
            Some(110),
            Some(95),
            "Final",
            "2025-11-01",
        );
        let standings = compute_standings(&[celtics, lakers], &[game], 2025);

        let bos = standings
            .eastern
            .iter()
            .find(|r| r.team.abbreviation == "BOS")
            .unwrap();
        let lal = standings
            .western
            .iter()
            .find(|r| r.team.abbreviation == "LAL")
            .unwrap();

        assert_eq!(bos.wins, 1);
        assert_eq!(bos.losses, 0);
        assert_eq!(lal.wins, 0);
        assert_eq!(lal.losses, 1);
    }

    // ── compute_standings: sorting ──────────────────────────────────

    #[test]
    fn teams_sorted_by_win_pct_descending() {
        let team_a = east_team(1, "TeamA", "AAA");
        let team_b = east_team(2, "TeamB", "BBB");
        let team_c = east_team(3, "TeamC", "CCC");

        // A beats B twice, C beats A once, B beats C once
        // A: 2-1 (.667), B: 1-2 (.333), C: 1-1 (.500)
        let games = vec![
            make_game(
                100,
                &team_a,
                &team_b,
                Some(110),
                Some(95),
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &team_a,
                &team_b,
                Some(105),
                Some(100),
                "Final",
                "2025-11-05",
            ),
            make_game(
                102,
                &team_c,
                &team_a,
                Some(120),
                Some(90),
                "Final",
                "2025-11-10",
            ),
            make_game(
                103,
                &team_b,
                &team_c,
                Some(100),
                Some(90),
                "Final",
                "2025-11-15",
            ),
        ];

        let standings = compute_standings(&[team_a, team_b, team_c], &games, 2025);

        assert_eq!(standings.eastern[0].team.abbreviation, "AAA"); // .667
        assert_eq!(standings.eastern[1].team.abbreviation, "CCC"); // .500
        assert_eq!(standings.eastern[2].team.abbreviation, "BBB"); // .333
    }

    #[test]
    fn tiebreaker_uses_total_wins() {
        let team_a = east_team(1, "TeamA", "AAA");
        let team_b = east_team(2, "TeamB", "BBB");
        let team_c = east_team(3, "TeamC", "CCC");

        // A: 2-2 (.500), B: 1-1 (.500), C: 1-1 (.500)
        // A has more wins (2) so should be ranked first among .500 teams
        let games = vec![
            make_game(
                100,
                &team_a,
                &team_b,
                Some(110),
                Some(95),
                "Final",
                "2025-11-01",
            ),
            make_game(
                101,
                &team_a,
                &team_c,
                Some(105),
                Some(100),
                "Final",
                "2025-11-05",
            ),
            make_game(
                102,
                &team_b,
                &team_a,
                Some(120),
                Some(90),
                "Final",
                "2025-11-10",
            ),
            make_game(
                103,
                &team_c,
                &team_a,
                Some(115),
                Some(105),
                "Final",
                "2025-11-15",
            ),
        ];

        let standings = compute_standings(&[team_a, team_b, team_c], &games, 2025);

        assert_eq!(standings.eastern[0].team.abbreviation, "AAA"); // .500 with 2 wins
    }

    // ── compute_standings: season passthrough ───────────────────────

    #[test]
    fn standings_carries_season() {
        let standings = compute_standings(&[], &[], 2025);
        assert_eq!(standings.season, 2025);
    }

    // ── compute_standings: empty inputs ─────────────────────────────

    #[test]
    fn no_teams_produces_empty_standings() {
        let standings = compute_standings(&[], &[], 2025);
        assert!(standings.eastern.is_empty());
        assert!(standings.western.is_empty());
    }

    #[test]
    fn no_teams_with_games_produces_empty_standings() {
        let phantom_a = east_team(1, "PhantomA", "PHA");
        let phantom_b = east_team(2, "PhantomB", "PHB");
        let game = make_game(
            100,
            &phantom_a,
            &phantom_b,
            Some(100),
            Some(90),
            "Final",
            "2025-11-01",
        );
        // teams list is empty, so games reference non-existent team IDs
        let standings = compute_standings(&[], &[game], 2025);
        assert!(standings.eastern.is_empty());
        assert!(standings.western.is_empty());
    }
}
