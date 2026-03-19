use serde::Deserialize;

/// Wrapper for paginated API responses from balldontlie.
#[derive(Debug, Deserialize)]
pub struct ApiResponse<T> {
    pub data: Vec<T>,
    pub meta: Option<Meta>,
}

/// Pagination metadata using cursor-based pagination.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Meta {
    pub next_cursor: Option<u64>,
    pub per_page: Option<u32>,
}

/// An NBA team as returned by the balldontlie API.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Team {
    pub id: u64,
    pub conference: String,
    pub division: String,
    pub city: String,
    pub name: String,
    pub full_name: String,
    pub abbreviation: String,
}

/// An NBA game as returned by the balldontlie API.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Game {
    pub id: u64,
    pub date: String,
    pub season: u32,
    pub status: String,
    pub period: Option<u32>,
    pub postseason: bool,
    pub postponed: Option<bool>,
    pub home_team_score: Option<u32>,
    pub visitor_team_score: Option<u32>,
    pub home_team: Team,
    pub visitor_team: Team,
}
