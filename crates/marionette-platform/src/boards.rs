//! Leaderboards and rich presence (decision 0215).
//!
//! Leaderboards answer later (stores answer over the network), so each request returns a
//! [`BoardRequest`] number and the answer arrives from [`Leaderboards::poll`] as a
//! [`BoardEvent`] carrying that number. Every request gets exactly one answer, also when the
//! board is missing or the store fails, so a game can always fall back to its own local best.
//!
//! Limits follow the strictest common store: names under [`MAX_BOARD_NAME`] bytes, at most
//! [`MAX_DETAILS`] detail integers per score.

use crate::{PlatformError, Result, codes};
use serde::{Deserialize, Serialize};

/// A board name must be shorter than this (bytes).
pub const MAX_BOARD_NAME: usize = 128;
/// Detail integers kept with one score.
pub const MAX_DETAILS: usize = 64;
/// Rich presence keys and values longer than this are refused (bytes).
pub const MAX_PRESENCE: usize = 256;

/// Which end of a board is best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    /// Lowest first (times).
    Ascending,
    /// Highest first (points, counts).
    Descending,
}

/// How a store shows scores.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScoreDisplay {
    Numeric,
    TimeSeconds,
    TimeMilliseconds,
}

/// Which rows to download.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowRange {
    /// Ranks `from..=to` (1 is the best).
    Global { from: i32, to: i32 },
    /// The player's own row with `before` rows above and `after` below.
    AroundPlayer { before: i32, after: i32 },
    /// The player and their friends.
    Friends,
}

/// One row of a board.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardRow {
    /// The player's store id.
    pub user: u64,
    /// The player's display name as the store has it.
    pub name: String,
    pub score: i32,
    /// 1 is the best.
    pub rank: i32,
    pub details: Vec<i32>,
}

/// The number that ties an answer to its request.
pub type BoardRequest = u64;

/// An answer.
#[derive(Clone, Debug, PartialEq)]
pub enum BoardEvent {
    /// The board exists (or was made). `found` is false when missing, refused or failed.
    Found {
        request: BoardRequest,
        board: String,
        found: bool,
    },
    /// A score went up. `rank` is the player's rank afterwards (0 when unknown or failed).
    Uploaded {
        request: BoardRequest,
        board: String,
        ok: bool,
        changed: bool,
        rank: i32,
    },
    /// Rows, best first; empty when the board is missing or the store failed.
    Rows {
        request: BoardRequest,
        board: String,
        rows: Vec<BoardRow>,
    },
}

/// Leaderboards on a platform.
pub trait Leaderboards {
    /// Looks a board up. With `create`, makes it when missing (stores may only allow that for
    /// the game's own app). The board's handle is kept, so later calls name it only.
    fn find(&mut self, board: &str, create: Option<(SortOrder, ScoreDisplay)>) -> BoardRequest;
    /// Sends a score to a board found before. `keep_best`: a worse score than the player's
    /// own best is not stored. `details` past [`MAX_DETAILS`] are dropped.
    fn upload(&mut self, board: &str, score: i32, keep_best: bool, details: &[i32]) -> BoardRequest;
    /// Downloads rows of a board found before.
    fn download(&mut self, board: &str, range: RowRange) -> BoardRequest;
    /// Answers that arrived since the last call.
    fn poll(&mut self) -> Vec<BoardEvent>;
}

/// What friends see the player doing (Steam rich presence). Keys are the store's.
pub trait Presence {
    fn set_presence(&mut self, key: &str, value: &str) -> Result<()>;
    fn clear_presence(&mut self);
}

/// Refuses board names a store would refuse.
pub fn check_board_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() >= MAX_BOARD_NAME {
        return Err(PlatformError::new(
            codes::NAME,
            format!(
                "board name of {} bytes: 1 to {} allowed",
                name.len(),
                MAX_BOARD_NAME - 1
            ),
        ));
    }
    Ok(())
}

/// Refuses presence text a store would refuse.
pub fn check_presence(key: &str, value: &str) -> Result<()> {
    if key.is_empty() || key.len() > MAX_PRESENCE || value.len() > MAX_PRESENCE {
        return Err(PlatformError::new(
            codes::NAME,
            format!("presence {key:?}: key 1 to {MAX_PRESENCE} bytes, value up to {MAX_PRESENCE}"),
        ));
    }
    Ok(())
}

/// One board as the offline backend keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct OfflineBoard {
    pub order: Option<SortOrder>,
    pub display: Option<ScoreDisplay>,
    pub rows: Vec<BoardRow>,
}

impl OfflineBoard {
    /// Stores the player's score; returns (changed, rank).
    pub fn put(&mut self, me: &BoardRow, keep_best: bool) -> (bool, i32) {
        let ascending = self.order == Some(SortOrder::Ascending);
        let better = |a: i32, b: i32| if ascending { a < b } else { a > b };
        let changed = match self.rows.iter_mut().find(|r| r.user == me.user) {
            Some(r) if keep_best && !better(me.score, r.score) => false,
            Some(r) => {
                r.score = me.score;
                r.details.clone_from(&me.details);
                true
            }
            None => {
                self.rows.push(me.clone());
                true
            }
        };
        // Stable: equal scores keep their first-come order.
        self.rows.sort_by(|a, b| {
            if ascending {
                a.score.cmp(&b.score)
            } else {
                b.score.cmp(&a.score)
            }
        });
        for (i, r) in self.rows.iter_mut().enumerate() {
            r.rank = i as i32 + 1;
        }
        let rank = self.rows.iter().find(|r| r.user == me.user).map_or(0, |r| r.rank);
        (changed, rank)
    }

    pub fn range(&self, me: u64, range: RowRange) -> Vec<BoardRow> {
        match range {
            RowRange::Global { from, to } => self
                .rows
                .iter()
                .filter(|r| r.rank >= from && r.rank <= to)
                .cloned()
                .collect(),
            RowRange::AroundPlayer { before, after } => {
                let Some(at) = self.rows.iter().position(|r| r.user == me) else {
                    return Vec::new();
                };
                let lo = at.saturating_sub(before.max(0) as usize);
                let hi = (at + after.max(0) as usize + 1).min(self.rows.len());
                self.rows[lo..hi].to_vec()
            }
            // Offline there are no friends: the player's own row.
            RowRange::Friends => self.rows.iter().filter(|r| r.user == me).cloned().collect(),
        }
    }
}
