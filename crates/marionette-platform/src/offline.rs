//! [`OfflinePlatform`]: achievements, stats and cloud files in a local folder.
//!
//! Layout: `<dir>/achievements.json` (unlocked ids and stats, written by `store`) and
//! `<dir>/cloud/<name>` (one file per cloud file). Writes go to a temporary name first and are
//! renamed into place, so a crash leaves the old or the new file, never half of one.
//! Leaderboards live in `<dir>/leaderboards.json` (written on each upload) with the player as
//! the only row-holder; presence is kept in memory only.

use crate::boards::{
    BoardEvent, BoardRequest, BoardRow, Leaderboards, MAX_DETAILS, OfflineBoard, Presence, RowRange,
    ScoreDisplay, SortOrder, check_board_name, check_presence,
};
use crate::{Achievements, CloudFiles, PlatformError, Result, StatValue, check_name, codes};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
struct State {
    unlocked: BTreeSet<String>,
    stats: BTreeMap<String, StatValue>,
}

pub struct OfflinePlatform {
    dir: PathBuf,
    state: State,
    cloud_enabled: bool,
    boards: Option<BTreeMap<String, OfflineBoard>>,
    found: BTreeSet<String>,
    answers: Vec<BoardEvent>,
    next_request: BoardRequest,
    presence: BTreeMap<String, String>,
    player: (u64, String),
}

fn io(path: &Path, e: std::io::Error) -> PlatformError {
    PlatformError::new(codes::IO, format!("{}: {e}", path.display()))
}

pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let tmp = path.with_extension("tmp-write");
    std::fs::write(&tmp, data).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| io(path, e))
}

impl OfflinePlatform {
    /// Opens (or starts) the platform folder `dir`.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        let path = dir.join("achievements.json");
        let state = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b)
                .map_err(|e| PlatformError::new(codes::IO, format!("{}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(io(&path, e)),
        };
        Ok(Self {
            dir,
            state,
            cloud_enabled: true,
            boards: None,
            found: BTreeSet::new(),
            answers: Vec::new(),
            next_request: 0,
            presence: BTreeMap::new(),
            player: (1, "Player".into()),
        })
    }

    /// The id and name the offline leaderboards file the player's rows under.
    pub fn set_player(&mut self, id: u64, name: &str) {
        self.player = (id, name.to_string());
    }

    /// What [`Presence`] was last given (offline nobody sees it; tests read it here).
    pub fn presence(&self) -> &BTreeMap<String, String> {
        &self.presence
    }

    fn boards(&mut self) -> &mut BTreeMap<String, OfflineBoard> {
        let dir = &self.dir;
        self.boards.get_or_insert_with(|| {
            // A missing or unreadable file starts empty boards (they are a local stand-in).
            std::fs::read(dir.join("leaderboards.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        })
    }

    fn save_boards(&mut self) -> Result<()> {
        let text = serde_json::to_vec_pretty(self.boards()).unwrap_or_default();
        write_atomic(&self.dir.join("leaderboards.json"), &text)
    }

    fn request(&mut self) -> BoardRequest {
        self.next_request += 1;
        self.next_request
    }

    /// Turns the simulated cloud off (as a player who disabled it).
    pub fn set_cloud_enabled(&mut self, on: bool) {
        self.cloud_enabled = on;
    }

    fn cloud_path(&self, name: &str) -> Result<PathBuf> {
        check_name(name)?;
        Ok(self.dir.join("cloud").join(name))
    }

    fn need_cloud(&self) -> Result<()> {
        if self.cloud_enabled {
            Ok(())
        } else {
            Err(PlatformError::new(codes::UNAVAILABLE, "cloud storage is off"))
        }
    }
}

impl Achievements for OfflinePlatform {
    fn unlock(&mut self, id: &str) -> Result<bool> {
        Ok(self.state.unlocked.insert(id.to_string()))
    }
    fn is_unlocked(&self, id: &str) -> Result<bool> {
        Ok(self.state.unlocked.contains(id))
    }
    fn clear(&mut self, id: &str) -> Result<()> {
        self.state.unlocked.remove(id);
        Ok(())
    }
    fn set_stat(&mut self, id: &str, value: StatValue) -> Result<()> {
        self.state.stats.insert(id.to_string(), value);
        Ok(())
    }
    fn stat(&self, id: &str) -> Result<Option<StatValue>> {
        Ok(self.state.stats.get(id).copied())
    }
    fn store(&mut self) -> Result<()> {
        let text = serde_json::to_vec_pretty(&self.state).unwrap_or_default();
        write_atomic(&self.dir.join("achievements.json"), &text)
    }
}

impl CloudFiles for OfflinePlatform {
    fn enabled(&self) -> bool {
        self.cloud_enabled
    }
    fn write(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.need_cloud()?;
        write_atomic(&self.cloud_path(name)?, data)
    }
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.need_cloud()?;
        let p = self.cloud_path(name)?;
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(&p, e)),
        }
    }
    fn delete(&mut self, name: &str) -> Result<()> {
        self.need_cloud()?;
        let p = self.cloud_path(name)?;
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io(&p, e)),
        }
    }
    fn modified(&self, name: &str) -> Result<Option<u64>> {
        self.need_cloud()?;
        let p = self.cloud_path(name)?;
        let meta = match std::fs::metadata(&p) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(&p, e)),
        };
        let at = meta.modified().map_err(|e| io(&p, e))?;
        Ok(at.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs()))
    }
    fn list(&self) -> Result<Vec<(String, u64)>> {
        self.need_cloud()?;
        let dir = self.dir.join("cloud");
        let mut out = Vec::new();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io(&dir, e)),
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp-write") {
                continue;
            }
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            out.push((name, len));
        }
        out.sort();
        Ok(out)
    }
}

impl Leaderboards for OfflinePlatform {
    fn find(&mut self, board: &str, create: Option<(SortOrder, ScoreDisplay)>) -> BoardRequest {
        let request = self.request();
        let found = check_board_name(board).is_ok() && {
            let boards = self.boards();
            match (boards.get_mut(board), create) {
                (Some(_), _) => true,
                (None, Some((order, display))) => {
                    boards.insert(
                        board.to_string(),
                        OfflineBoard {
                            order: Some(order),
                            display: Some(display),
                            rows: Vec::new(),
                        },
                    );
                    true
                }
                (None, None) => false,
            }
        };
        if found {
            self.found.insert(board.to_string());
            if create.is_some() && self.save_boards().is_err() {
                // Kept in memory for this run; the file is only a convenience offline.
            }
        }
        self.answers.push(BoardEvent::Found {
            request,
            board: board.to_string(),
            found,
        });
        request
    }

    fn upload(&mut self, board: &str, score: i32, keep_best: bool, details: &[i32]) -> BoardRequest {
        let request = self.request();
        let me = BoardRow {
            user: self.player.0,
            name: self.player.1.clone(),
            score,
            rank: 0,
            details: details.iter().take(MAX_DETAILS).copied().collect(),
        };
        let known = self.found.contains(board);
        let put = if known {
            self.boards().get_mut(board).map(|b| b.put(&me, keep_best))
        } else {
            None
        };
        let event = match put {
            Some((changed, rank)) => BoardEvent::Uploaded {
                request,
                board: board.to_string(),
                ok: self.save_boards().is_ok(),
                changed,
                rank,
            },
            None => BoardEvent::Uploaded {
                request,
                board: board.to_string(),
                ok: false,
                changed: false,
                rank: 0,
            },
        };
        self.answers.push(event);
        request
    }

    fn download(&mut self, board: &str, range: RowRange) -> BoardRequest {
        let request = self.request();
        let me = self.player.0;
        let rows = if self.found.contains(board) {
            self.boards()
                .get(board)
                .map(|b| b.range(me, range))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.answers.push(BoardEvent::Rows {
            request,
            board: board.to_string(),
            rows,
        });
        request
    }

    fn poll(&mut self) -> Vec<BoardEvent> {
        std::mem::take(&mut self.answers)
    }
}

impl Presence for OfflinePlatform {
    fn set_presence(&mut self, key: &str, value: &str) -> Result<()> {
        check_presence(key, value)?;
        self.presence.insert(key.to_string(), value.to_string());
        Ok(())
    }
    fn clear_presence(&mut self) {
        self.presence.clear();
    }
}
