//! Lobbies: find and gather players before a session. The [`Lobbies`] trait is what a game
//! uses; [`LocalLobbies`] is an in-process directory for tests and local play, the Steam
//! backend uses Steam matchmaking.
//!
//! Requests complete later: call [`Lobbies::poll`] every frame and act on the events (the
//! Steam API works the same way).

use crate::transport::{NetError, PeerId, codes};
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct LobbyId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    /// Listed in searches.
    Public,
    /// Listed for friends only (where the backend knows friends; elsewhere like `Public`).
    FriendsOnly,
    /// Joined by id or invite only.
    Private,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LobbyInfo {
    pub id: LobbyId,
    pub owner: PeerId,
    /// In join order.
    pub members: Vec<PeerId>,
    pub max_members: u32,
    pub data: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LobbyEvent {
    Created(LobbyId),
    Joined(LobbyId),
    JoinFailed {
        lobby: LobbyId,
        reason: String,
    },
    /// Search results, best first.
    List(Vec<LobbyId>),
    /// Members, owner or data changed.
    Changed(LobbyId),
}

pub trait Lobbies {
    /// Creates a lobby owned by this player and joins it; [`LobbyEvent::Created`] follows.
    fn create(&mut self, max_members: u32, visibility: Visibility);
    fn join(&mut self, lobby: LobbyId);
    fn leave(&mut self, lobby: LobbyId);
    /// Searches visible lobbies whose data has every `(key, value)`; [`LobbyEvent::List`].
    fn request_list(&mut self, filters: &[(String, String)]);
    /// Sets lobby data (owner only).
    fn set_data(&mut self, lobby: LobbyId, key: &str, value: &str) -> Result<(), NetError>;
    fn info(&self, lobby: LobbyId) -> Option<LobbyInfo>;
    fn poll(&mut self) -> Vec<LobbyEvent>;
}

struct Entry {
    info: LobbyInfo,
    visibility: Visibility,
}

#[derive(Default)]
struct Dir {
    next: u64,
    lobbies: BTreeMap<LobbyId, Entry>,
    queues: BTreeMap<PeerId, VecDeque<LobbyEvent>>,
}

impl Dir {
    fn tell_members(&mut self, id: LobbyId) {
        let members = self
            .lobbies
            .get(&id)
            .map(|e| e.info.members.clone())
            .unwrap_or_default();
        for m in members {
            self.queues
                .entry(m)
                .or_default()
                .push_back(LobbyEvent::Changed(id));
        }
    }
}

/// An in-process lobby directory. Clone the directory handle with [`LocalLobbies::handle`]
/// for each player.
#[derive(Clone)]
pub struct LocalLobbies {
    dir: Arc<Mutex<Dir>>,
    me: PeerId,
}

impl LocalLobbies {
    /// A new, empty directory, used by player `me`.
    pub fn new(me: PeerId) -> Self {
        Self {
            dir: Arc::new(Mutex::new(Dir {
                next: 1,
                ..Dir::default()
            })),
            me,
        }
    }

    /// The same directory, used by player `me`.
    pub fn handle(&self, me: PeerId) -> Self {
        Self {
            dir: self.dir.clone(),
            me,
        }
    }

    fn dir(&self) -> std::sync::MutexGuard<'_, Dir> {
        self.dir.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(d: &mut Dir, to: PeerId, e: LobbyEvent) {
        d.queues.entry(to).or_default().push_back(e);
    }
}

impl Lobbies for LocalLobbies {
    fn create(&mut self, max_members: u32, visibility: Visibility) {
        let me = self.me;
        let mut d = self.dir();
        let id = LobbyId(d.next);
        d.next += 1;
        d.lobbies.insert(
            id,
            Entry {
                info: LobbyInfo {
                    id,
                    owner: me,
                    members: vec![me],
                    max_members: max_members.max(1),
                    data: BTreeMap::new(),
                },
                visibility,
            },
        );
        Self::push(&mut d, me, LobbyEvent::Created(id));
    }

    fn join(&mut self, lobby: LobbyId) {
        let me = self.me;
        let mut d = self.dir();
        let result = match d.lobbies.get_mut(&lobby) {
            None => Err("no such lobby"),
            Some(e) if e.info.members.contains(&me) => Ok(false),
            Some(e) if e.info.members.len() as u32 >= e.info.max_members => Err("lobby is full"),
            Some(e) => {
                e.info.members.push(me);
                Ok(true)
            }
        };
        match result {
            Ok(changed) => {
                if changed {
                    d.tell_members(lobby);
                }
                Self::push(&mut d, me, LobbyEvent::Joined(lobby));
            }
            Err(reason) => Self::push(
                &mut d,
                me,
                LobbyEvent::JoinFailed {
                    lobby,
                    reason: reason.into(),
                },
            ),
        }
    }

    fn leave(&mut self, lobby: LobbyId) {
        let me = self.me;
        let mut d = self.dir();
        let Some(e) = d.lobbies.get_mut(&lobby) else {
            return;
        };
        e.info.members.retain(|m| *m != me);
        if e.info.members.is_empty() {
            d.lobbies.remove(&lobby);
            return;
        }
        if e.info.owner == me {
            // Like Steam: the longest-standing member becomes the owner.
            e.info.owner = e.info.members[0];
        }
        d.tell_members(lobby);
    }

    fn request_list(&mut self, filters: &[(String, String)]) {
        let me = self.me;
        let mut d = self.dir();
        let ids = d
            .lobbies
            .values()
            .filter(|e| e.visibility != Visibility::Private)
            .filter(|e| (e.info.members.len() as u32) < e.info.max_members)
            .filter(|e| filters.iter().all(|(k, v)| e.info.data.get(k) == Some(v)))
            .map(|e| e.info.id)
            .collect();
        Self::push(&mut d, me, LobbyEvent::List(ids));
    }

    fn set_data(&mut self, lobby: LobbyId, key: &str, value: &str) -> Result<(), NetError> {
        let me = self.me;
        let mut d = self.dir();
        let e = d
            .lobbies
            .get_mut(&lobby)
            .ok_or_else(|| NetError::new(codes::LOBBY, format!("no lobby {}", lobby.0)))?;
        if e.info.owner != me {
            return Err(NetError::new(codes::LOBBY, "only the owner sets lobby data"));
        }
        e.info.data.insert(key.into(), value.into());
        d.tell_members(lobby);
        Ok(())
    }

    fn info(&self, lobby: LobbyId) -> Option<LobbyInfo> {
        self.dir().lobbies.get(&lobby).map(|e| e.info.clone())
    }

    fn poll(&mut self) -> Vec<LobbyEvent> {
        let me = self.me;
        self.dir()
            .queues
            .get_mut(&me)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_find_join_leave() {
        let mut a = LocalLobbies::new(PeerId(1));
        let mut b = a.handle(PeerId(2));
        let mut c = a.handle(PeerId(3));
        a.create(2, Visibility::Public);
        let LobbyEvent::Created(id) = a.poll()[0] else {
            panic!()
        };
        a.set_data(id, "mode", "coop").unwrap();
        assert!(b.set_data(id, "mode", "x").is_err(), "only the owner");
        b.request_list(&[("mode".into(), "coop".into())]);
        assert_eq!(b.poll(), vec![LobbyEvent::List(vec![id])]);
        b.join(id);
        assert_eq!(b.poll(), vec![LobbyEvent::Changed(id), LobbyEvent::Joined(id)]);
        c.join(id);
        assert!(matches!(c.poll()[0], LobbyEvent::JoinFailed { .. }), "full");
        a.leave(id);
        assert_eq!(b.info(id).unwrap().owner, PeerId(2), "ownership moves");
    }
}
