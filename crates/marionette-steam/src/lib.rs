//! Optional Steam backend for Marionette (decision 0212).
//!
//! Implements the engine's interfaces on Steam: [`marionette_net::Transport`] over Steam
//! networking messages (peer to peer through Valve's relays, NAT traversal included),
//! [`marionette_net::Lobbies`] over Steam matchmaking, and
//! [`marionette_platform::Achievements`] and [`marionette_platform::CloudFiles`] over user
//! stats and remote storage.
//!
//! No Steamworks SDK file is in this repository. The game ships the redistributable
//! `steam_api64.dll` / `libsteam_api.so` / `libsteam_api.dylib` from its own SDK download
//! (allowed by the Steamworks agreement the game's developer accepts), and this crate loads
//! it at run time. Without the library or without a running Steam client, [`Steam::load`]
//! returns an error and the game falls back to the offline platform and UDP or loopback.
//!
//! During development put `steam_appid.txt` (the game's app id) next to the executable, or
//! launch through Steam.

mod ffi;

use ffi::{CallbackMsg, Fns, Ptr, id, offset};
use libloading::Library;
use marionette_net::{
    Delivery, LobbyEvent, LobbyId, LobbyInfo, Micros, NetError, Packet, PeerId, Transport, TransportStats,
    Visibility,
};
use marionette_platform::{PlatformError, StatValue};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CStr, CString, c_char};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

/// Stable error codes (`steam.*`).
pub mod codes {
    /// The steam_api library is missing or lacks a needed function.
    pub const LIBRARY: &str = "steam.library";
    /// Steam did not start (client not running, no app id, wrong user).
    pub const INIT: &str = "steam.init";
    /// A Steam call failed.
    pub const CALL: &str = "steam.call";
}

#[derive(Clone, Debug, PartialEq)]
pub struct SteamError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for SteamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SteamError {}

fn err(code: &'static str, message: impl Into<String>) -> SteamError {
    SteamError {
        code,
        message: message.into(),
    }
}

/// The library file name Steam uses on this OS.
pub fn default_library() -> &'static str {
    if cfg!(windows) {
        "steam_api64.dll"
    } else if cfg!(target_os = "macos") {
        "libsteam_api.dylib"
    } else {
        "libsteam_api.so"
    }
}

#[derive(Clone, Copy)]
enum Call {
    Create,
    Join(LobbyId),
    List,
}

struct Ifaces {
    user: Ptr,
    utils: Ptr,
    matchmaking: Ptr,
    stats: Ptr,
    storage: Ptr,
    messages: Ptr,
}

struct Inner {
    f: Fns,
    i: Ifaces,
    pipe: i32,
    calls: BTreeMap<u64, Call>,
    lobby_events: VecDeque<LobbyEvent>,
    shut: bool,
    // Declared last so it is dropped after everything that points into it.
    _lib: Library,
}

// The flat API is called only while holding the mutex, one thread at a time.
unsafe impl Send for Inner {}

impl Drop for Inner {
    fn drop(&mut self) {
        if !self.shut {
            self.shut = true;
            unsafe { (self.f.SteamAPI_Shutdown)() };
        }
    }
}

/// A running Steam API. Cheap to clone; the API shuts down when the last clone is dropped.
#[derive(Clone)]
pub struct Steam(Arc<Mutex<Inner>>);

/// What [`Steam::load`] found (for logs and the `steam_check` example).
#[derive(Clone, Debug)]
pub struct SteamInfo {
    pub library: PathBuf,
    pub app_id: u32,
    pub user: u64,
}

impl Steam {
    /// Loads the steam_api library (`library`, or [`default_library`] next to the executable
    /// and on the system search path), starts the Steam API and switches it to manual
    /// callback dispatch.
    pub fn load(library: Option<&Path>) -> Result<(Steam, SteamInfo), SteamError> {
        let path = match library {
            Some(p) => p.to_path_buf(),
            None => std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(|d| d.join(default_library())))
                .filter(|p| p.exists())
                .unwrap_or_else(|| PathBuf::from(default_library())),
        };
        // Loading runs the library's initialisers: that is the point of an optional backend
        // the game opts into with its own copy of the library.
        let lib = unsafe { Library::new(&path) }
            .map_err(|e| err(codes::LIBRARY, format!("cannot load {}: {e}", path.display())))?;
        let f = unsafe { Fns::load(&lib) }.map_err(|e| {
            err(
                codes::LIBRARY,
                format!(
                    "{} lacks a needed function ({e}); use a newer Steamworks SDK",
                    path.display()
                ),
            )
        })?;
        unsafe { init(&lib) }?;
        let iface = |names| unsafe { ffi::interface(&lib, names) }.map_err(|e| err(codes::INIT, e));
        let i = Ifaces {
            user: iface(ffi::USER)?,
            utils: iface(ffi::UTILS)?,
            matchmaking: iface(ffi::MATCHMAKING)?,
            stats: iface(ffi::USER_STATS)?,
            storage: iface(ffi::REMOTE_STORAGE)?,
            messages: iface(ffi::NET_MESSAGES)?,
        };
        unsafe {
            (f.SteamAPI_ManualDispatch_Init)();
            if let Ok(utils) = ffi::interface(&lib, ffi::NET_UTILS) {
                (f.SteamAPI_ISteamNetworkingUtils_InitRelayNetworkAccess)(utils);
            }
            // Libraries older than SDK 1.61 load stats only on request.
            if let Ok(req) = lib
                .get::<unsafe extern "C" fn(Ptr) -> bool>(b"SteamAPI_ISteamUserStats_RequestCurrentStats\0")
            {
                req(i.stats);
            }
        }
        let pipe = unsafe { (f.SteamAPI_GetHSteamPipe)() };
        let info = SteamInfo {
            library: path,
            app_id: unsafe { (f.SteamAPI_ISteamUtils_GetAppID)(i.utils) },
            user: unsafe { (f.SteamAPI_ISteamUser_GetSteamID)(i.user) },
        };
        let inner = Inner {
            f,
            i,
            pipe,
            calls: BTreeMap::new(),
            lobby_events: VecDeque::new(),
            shut: false,
            _lib: lib,
        };
        Ok((Steam(Arc::new(Mutex::new(inner))), info))
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether the Steam client is still running.
    pub fn is_running(&self) -> bool {
        let s = self.inner();
        unsafe { (s.f.SteamAPI_IsSteamRunning)() }
    }

    /// This player's Steam ID as a peer id.
    pub fn me(&self) -> PeerId {
        let s = self.inner();
        PeerId(unsafe { (s.f.SteamAPI_ISteamUser_GetSteamID)(s.i.user) })
    }

    /// Runs Steam's callbacks (lobby results, connection requests). Call once per frame;
    /// [`SteamTransport::poll`] and [`SteamLobbies::poll`] call it too.
    pub fn run_callbacks(&self) {
        self.inner().dispatch();
    }

    pub fn transport(&self, channels: u8) -> SteamTransport {
        SteamTransport {
            steam: self.clone(),
            channels: channels.max(1),
            received: VecDeque::new(),
            partial: BTreeMap::new(),
            stats: TransportStats::default(),
        }
    }

    pub fn lobbies(&self) -> SteamLobbies {
        SteamLobbies { steam: self.clone() }
    }

    pub fn platform(&self) -> SteamPlatform {
        SteamPlatform { steam: self.clone() }
    }
}

/// Starts the API: `SteamAPI_InitFlat` (SDK 1.59 and later), else the older `SteamAPI_Init`.
unsafe fn init(lib: &Library) -> Result<(), SteamError> {
    unsafe {
        if let Ok(flat) = lib.get::<unsafe extern "C" fn(*mut [c_char; 1024]) -> i32>(b"SteamAPI_InitFlat\0")
        {
            let mut msg = [0 as c_char; 1024];
            let r = flat(&mut msg);
            if r != 0 {
                let text = CStr::from_ptr(msg.as_ptr()).to_string_lossy().into_owned();
                return Err(err(
                    codes::INIT,
                    format!("SteamAPI_InitFlat returned {r}: {text}"),
                ));
            }
            return Ok(());
        }
        let old = lib
            .get::<unsafe extern "C" fn() -> bool>(b"SteamAPI_Init\0")
            .map_err(|e| {
                err(
                    codes::LIBRARY,
                    format!("no SteamAPI_InitFlat or SteamAPI_Init: {e}"),
                )
            })?;
        if old() {
            Ok(())
        } else {
            Err(err(
                codes::INIT,
                "SteamAPI_Init failed: is Steam running, and is steam_appid.txt next to the game?",
            ))
        }
    }
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).expect("no NUL left")
}

impl Inner {
    fn identity(&self, peer: PeerId) -> [u8; id::IDENTITY_SIZE] {
        let mut ident = [0u8; id::IDENTITY_SIZE];
        unsafe { (self.f.SteamAPI_SteamNetworkingIdentity_SetSteamID64)(ident.as_mut_ptr().cast(), peer.0) };
        ident
    }

    fn dispatch(&mut self) {
        unsafe {
            (self.f.SteamAPI_ManualDispatch_RunFrame)(self.pipe);
            let mut msg = std::mem::zeroed::<CallbackMsg>();
            while (self.f.SteamAPI_ManualDispatch_GetNextCallback)(self.pipe, &mut msg) {
                let param = msg.param;
                match msg.callback {
                    id::API_CALL_COMPLETED => {
                        let handle = std::ptr::read_unaligned(param.cast::<u64>());
                        let expected = std::ptr::read_unaligned(param.add(8).cast::<i32>());
                        let size = std::ptr::read_unaligned(param.add(12).cast::<u32>()) as usize;
                        if let Some(call) = self.calls.remove(&handle) {
                            let mut buf = vec![0u8; size.max(32)];
                            let mut failed = false;
                            let ok = (self.f.SteamAPI_ManualDispatch_GetAPICallResult)(
                                self.pipe,
                                handle,
                                buf.as_mut_ptr().cast(),
                                size as i32,
                                expected,
                                &mut failed,
                            );
                            self.call_done(call, expected, ok && !failed, &buf);
                        }
                    }
                    id::NET_SESSION_REQUEST => {
                        // Lobby members and anyone we message may talk to us; the session
                        // layer decides what to do with them.
                        (self.f.SteamAPI_ISteamNetworkingMessages_AcceptSessionWithUser)(
                            self.i.messages,
                            param.cast(),
                        );
                    }
                    id::LOBBY_DATA_UPDATE | id::LOBBY_CHAT_UPDATE => {
                        let lobby = std::ptr::read_unaligned(param.cast::<u64>());
                        self.lobby_events.push_back(LobbyEvent::Changed(LobbyId(lobby)));
                    }
                    _ => {}
                }
                (self.f.SteamAPI_ManualDispatch_FreeLastCallback)(self.pipe);
            }
        }
    }

    fn call_done(&mut self, call: Call, expected: i32, ok: bool, buf: &[u8]) {
        let read_u64 = |at: usize| u64::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes"));
        let read_u32 = |at: usize| u32::from_le_bytes(buf[at..at + 4].try_into().expect("4 bytes"));
        match (call, expected) {
            (Call::Create, id::LOBBY_CREATED) => {
                let result = read_u32(0) as i32;
                if ok && result == id::RESULT_OK {
                    let lobby = LobbyId(read_u64(offset::LOBBY_CREATED_ID));
                    self.lobby_events.push_back(LobbyEvent::Created(lobby));
                } else {
                    self.lobby_events.push_back(LobbyEvent::JoinFailed {
                        lobby: LobbyId(0),
                        reason: format!("lobby not created (result {result})"),
                    });
                }
            }
            (Call::Join(lobby), id::LOBBY_ENTER) => {
                // LobbyEnter_t: id u64 @0, permissions u32 @8, locked bool @12, response u32 @16.
                let response = read_u32(16);
                if ok && response == id::CHAT_ENTER_SUCCESS {
                    self.lobby_events.push_back(LobbyEvent::Joined(lobby));
                } else {
                    self.lobby_events.push_back(LobbyEvent::JoinFailed {
                        lobby,
                        reason: format!("could not enter the lobby (response {response})"),
                    });
                }
            }
            (Call::List, id::LOBBY_MATCH_LIST) => {
                let n = if ok { read_u32(0) } else { 0 };
                let ids = (0..n as i32)
                    .map(|i| {
                        LobbyId(unsafe {
                            (self.f.SteamAPI_ISteamMatchmaking_GetLobbyByIndex)(self.i.matchmaking, i)
                        })
                    })
                    .collect();
                self.lobby_events.push_back(LobbyEvent::List(ids));
            }
            _ => {}
        }
    }
}

/// [`Transport`] over Steam networking messages. Reliable messages larger than Steam's
/// 512 KiB limit are split and rebuilt; every message carries one leading byte (bit 0: more
/// parts follow, bit 1: sent reliably).
pub struct SteamTransport {
    steam: Steam,
    channels: u8,
    received: VecDeque<Packet>,
    partial: BTreeMap<(PeerId, u8), Vec<u8>>,
    stats: TransportStats,
}

impl Transport for SteamTransport {
    fn local(&self) -> PeerId {
        self.steam.me()
    }

    fn send(&mut self, to: PeerId, delivery: Delivery, channel: u8, data: &[u8]) -> Result<(), NetError> {
        let s = self.steam.inner();
        let ident = s.identity(to);
        let (flags, part) = match delivery {
            Delivery::Unreliable => {
                if data.len() > marionette_net::MAX_UNRELIABLE {
                    return Err(NetError::new(
                        marionette_net::codes::TOO_LARGE,
                        "unreliable message too large",
                    ));
                }
                (
                    id::SEND_UNRELIABLE_NO_NAGLE | id::SEND_AUTO_RESTART,
                    data.len().max(1),
                )
            }
            Delivery::Reliable => (id::SEND_RELIABLE | id::SEND_AUTO_RESTART, id::MAX_MESSAGE - 1),
        };
        let chunks: Vec<&[u8]> = if data.is_empty() {
            vec![data]
        } else {
            data.chunks(part).collect()
        };
        let last = chunks.len() - 1;
        for (i, c) in chunks.iter().enumerate() {
            let mut msg = Vec::with_capacity(c.len() + 1);
            msg.push(u8::from(i < last) | if delivery == Delivery::Reliable { 2 } else { 0 });
            msg.extend_from_slice(c);
            let r = unsafe {
                (s.f.SteamAPI_ISteamNetworkingMessages_SendMessageToUser)(
                    s.i.messages,
                    ident.as_ptr().cast(),
                    msg.as_ptr().cast(),
                    msg.len() as u32,
                    flags,
                    channel as i32,
                )
            };
            self.stats.bytes_sent += msg.len() as u64;
            self.stats.datagrams_sent += 1;
            if r != id::RESULT_OK && delivery == Delivery::Reliable {
                return Err(NetError::new(
                    marionette_net::codes::IO,
                    format!("SendMessageToUser to {to} returned {r}"),
                ));
            }
        }
        Ok(())
    }

    fn poll(&mut self, _now: Micros) {
        let mut s = self.steam.inner();
        s.dispatch();
        let mut msgs: [Ptr; 64] = [std::ptr::null_mut(); 64];
        for ch in 0..self.channels {
            loop {
                let n = unsafe {
                    (s.f.SteamAPI_ISteamNetworkingMessages_ReceiveMessagesOnChannel)(
                        s.i.messages,
                        ch as i32,
                        msgs.as_mut_ptr(),
                        msgs.len() as i32,
                    )
                };
                if n <= 0 {
                    break;
                }
                for &m in &msgs[..n as usize] {
                    let (data, from) = unsafe {
                        let base = m.cast::<u8>();
                        let ptr = std::ptr::read_unaligned(base.add(offset::MSG_DATA).cast::<*const u8>());
                        let len = std::ptr::read_unaligned(base.add(offset::MSG_SIZE).cast::<i32>()).max(0)
                            as usize;
                        let data = std::slice::from_raw_parts(ptr, len).to_vec();
                        let from = (s.f.SteamAPI_SteamNetworkingIdentity_GetSteamID64)(
                            base.add(offset::MSG_PEER).cast(),
                        );
                        (s.f.SteamAPI_SteamNetworkingMessage_t_Release)(m);
                        (data, PeerId(from))
                    };
                    self.stats.bytes_received += data.len() as u64;
                    let Some((&head, body)) = data.split_first() else {
                        continue;
                    };
                    let buf = self.partial.entry((from, ch)).or_default();
                    buf.extend_from_slice(body);
                    if head & 1 == 0 {
                        let whole = std::mem::take(buf);
                        self.partial.remove(&(from, ch));
                        let delivery = if head & 2 != 0 {
                            Delivery::Reliable
                        } else {
                            Delivery::Unreliable
                        };
                        self.received.push_back(Packet {
                            from,
                            channel: ch,
                            delivery,
                            data: whole,
                        });
                    }
                }
                if (n as usize) < msgs.len() {
                    break;
                }
            }
        }
    }

    fn receive(&mut self) -> Option<Packet> {
        self.received.pop_front()
    }

    fn stats(&self) -> TransportStats {
        self.stats
    }
}

/// [`marionette_net::Lobbies`] over Steam matchmaking.
pub struct SteamLobbies {
    steam: Steam,
}

impl marionette_net::Lobbies for SteamLobbies {
    fn create(&mut self, max_members: u32, visibility: Visibility) {
        let mut s = self.steam.inner();
        let kind = match visibility {
            Visibility::Public => id::LOBBY_PUBLIC,
            Visibility::FriendsOnly => id::LOBBY_FRIENDS_ONLY,
            Visibility::Private => id::LOBBY_PRIVATE,
        };
        let h = unsafe {
            (s.f.SteamAPI_ISteamMatchmaking_CreateLobby)(s.i.matchmaking, kind, max_members as i32)
        };
        s.calls.insert(h, Call::Create);
    }

    fn join(&mut self, lobby: LobbyId) {
        let mut s = self.steam.inner();
        let h = unsafe { (s.f.SteamAPI_ISteamMatchmaking_JoinLobby)(s.i.matchmaking, lobby.0) };
        s.calls.insert(h, Call::Join(lobby));
    }

    fn leave(&mut self, lobby: LobbyId) {
        let s = self.steam.inner();
        unsafe { (s.f.SteamAPI_ISteamMatchmaking_LeaveLobby)(s.i.matchmaking, lobby.0) };
    }

    fn request_list(&mut self, filters: &[(String, String)]) {
        let mut s = self.steam.inner();
        for (k, v) in filters {
            let (k, v) = (cstr(k), cstr(v));
            unsafe {
                (s.f.SteamAPI_ISteamMatchmaking_AddRequestLobbyListStringFilter)(
                    s.i.matchmaking,
                    k.as_ptr(),
                    v.as_ptr(),
                    id::COMPARE_EQUAL,
                )
            };
        }
        let h = unsafe { (s.f.SteamAPI_ISteamMatchmaking_RequestLobbyList)(s.i.matchmaking) };
        s.calls.insert(h, Call::List);
    }

    fn set_data(&mut self, lobby: LobbyId, key: &str, value: &str) -> Result<(), NetError> {
        let s = self.steam.inner();
        let (k, v) = (cstr(key), cstr(value));
        let ok = unsafe {
            (s.f.SteamAPI_ISteamMatchmaking_SetLobbyData)(s.i.matchmaking, lobby.0, k.as_ptr(), v.as_ptr())
        };
        if ok {
            Ok(())
        } else {
            Err(NetError::new(
                marionette_net::codes::LOBBY,
                "SetLobbyData refused (not the owner?)",
            ))
        }
    }

    fn info(&self, lobby: LobbyId) -> Option<LobbyInfo> {
        let s = self.steam.inner();
        let mm = s.i.matchmaking;
        unsafe {
            let n = (s.f.SteamAPI_ISteamMatchmaking_GetNumLobbyMembers)(mm, lobby.0);
            if n <= 0 {
                return None;
            }
            let members = (0..n)
                .map(|i| {
                    PeerId((s.f.SteamAPI_ISteamMatchmaking_GetLobbyMemberByIndex)(
                        mm, lobby.0, i,
                    ))
                })
                .collect();
            let mut data = BTreeMap::new();
            for i in 0..(s.f.SteamAPI_ISteamMatchmaking_GetLobbyDataCount)(mm, lobby.0) {
                let mut k = [0 as c_char; 256];
                let mut v = [0 as c_char; 8192];
                if (s.f.SteamAPI_ISteamMatchmaking_GetLobbyDataByIndex)(
                    mm,
                    lobby.0,
                    i,
                    k.as_mut_ptr(),
                    k.len() as i32,
                    v.as_mut_ptr(),
                    v.len() as i32,
                ) {
                    data.insert(
                        CStr::from_ptr(k.as_ptr()).to_string_lossy().into_owned(),
                        CStr::from_ptr(v.as_ptr()).to_string_lossy().into_owned(),
                    );
                }
            }
            Some(LobbyInfo {
                id: lobby,
                owner: PeerId((s.f.SteamAPI_ISteamMatchmaking_GetLobbyOwner)(mm, lobby.0)),
                members,
                max_members: (s.f.SteamAPI_ISteamMatchmaking_GetLobbyMemberLimit)(mm, lobby.0).max(0) as u32,
                data,
            })
        }
    }

    fn poll(&mut self) -> Vec<LobbyEvent> {
        let mut s = self.steam.inner();
        s.dispatch();
        s.lobby_events.drain(..).collect()
    }
}

/// Achievements, stats and cloud files on Steam. Achievement and stat ids are the API names
/// set up on the game's Steamworks page.
pub struct SteamPlatform {
    steam: Steam,
}

fn perr(code: &'static str, msg: impl Into<String>) -> PlatformError {
    PlatformError::new(code, msg)
}

impl marionette_platform::Achievements for SteamPlatform {
    fn unlock(&mut self, id: &str) -> marionette_platform::Result<bool> {
        let was = self.is_unlocked(id)?;
        let s = self.steam.inner();
        let c = cstr(id);
        if unsafe { (s.f.SteamAPI_ISteamUserStats_SetAchievement)(s.i.stats, c.as_ptr()) } {
            Ok(!was)
        } else {
            Err(perr(
                marionette_platform::codes::UNKNOWN,
                format!("Steam refused achievement {id:?}"),
            ))
        }
    }

    fn is_unlocked(&self, id: &str) -> marionette_platform::Result<bool> {
        let s = self.steam.inner();
        let c = cstr(id);
        let mut got = false;
        if unsafe { (s.f.SteamAPI_ISteamUserStats_GetAchievement)(s.i.stats, c.as_ptr(), &mut got) } {
            Ok(got)
        } else {
            Err(perr(
                marionette_platform::codes::UNKNOWN,
                format!("Steam has no achievement {id:?}"),
            ))
        }
    }

    fn clear(&mut self, id: &str) -> marionette_platform::Result<()> {
        let s = self.steam.inner();
        let c = cstr(id);
        unsafe { (s.f.SteamAPI_ISteamUserStats_ClearAchievement)(s.i.stats, c.as_ptr()) };
        Ok(())
    }

    fn set_stat(&mut self, id: &str, value: StatValue) -> marionette_platform::Result<()> {
        let s = self.steam.inner();
        let c = cstr(id);
        let ok = unsafe {
            match value {
                StatValue::Int(v) => (s.f.SteamAPI_ISteamUserStats_SetStatInt32)(s.i.stats, c.as_ptr(), v),
                StatValue::Float(v) => (s.f.SteamAPI_ISteamUserStats_SetStatFloat)(s.i.stats, c.as_ptr(), v),
            }
        };
        if ok {
            Ok(())
        } else {
            Err(perr(
                marionette_platform::codes::UNKNOWN,
                format!("Steam refused stat {id:?}"),
            ))
        }
    }

    fn stat(&self, id: &str) -> marionette_platform::Result<Option<StatValue>> {
        let s = self.steam.inner();
        let c = cstr(id);
        unsafe {
            let mut i = 0i32;
            if (s.f.SteamAPI_ISteamUserStats_GetStatInt32)(s.i.stats, c.as_ptr(), &mut i) {
                return Ok(Some(StatValue::Int(i)));
            }
            let mut f = 0f32;
            if (s.f.SteamAPI_ISteamUserStats_GetStatFloat)(s.i.stats, c.as_ptr(), &mut f) {
                return Ok(Some(StatValue::Float(f)));
            }
        }
        Ok(None)
    }

    fn show_progress(&mut self, id: &str, current: u32, max: u32) -> marionette_platform::Result<()> {
        let s = self.steam.inner();
        let c = cstr(id);
        unsafe {
            (s.f.SteamAPI_ISteamUserStats_IndicateAchievementProgress)(s.i.stats, c.as_ptr(), current, max)
        };
        Ok(())
    }

    fn store(&mut self) -> marionette_platform::Result<()> {
        let s = self.steam.inner();
        if unsafe { (s.f.SteamAPI_ISteamUserStats_StoreStats)(s.i.stats) } {
            Ok(())
        } else {
            Err(perr(
                marionette_platform::codes::UNAVAILABLE,
                "StoreStats failed (stats not loaded yet?)",
            ))
        }
    }
}

impl marionette_platform::CloudFiles for SteamPlatform {
    fn enabled(&self) -> bool {
        let s = self.steam.inner();
        unsafe {
            (s.f.SteamAPI_ISteamRemoteStorage_IsCloudEnabledForAccount)(s.i.storage)
                && (s.f.SteamAPI_ISteamRemoteStorage_IsCloudEnabledForApp)(s.i.storage)
        }
    }

    fn write(&mut self, name: &str, data: &[u8]) -> marionette_platform::Result<()> {
        marionette_platform::check_name(name)?;
        let s = self.steam.inner();
        let c = cstr(name);
        if unsafe {
            (s.f.SteamAPI_ISteamRemoteStorage_FileWrite)(
                s.i.storage,
                c.as_ptr(),
                data.as_ptr().cast(),
                data.len() as i32,
            )
        } {
            Ok(())
        } else {
            Err(perr(
                marionette_platform::codes::UNAVAILABLE,
                format!("FileWrite {name:?} failed (quota?)"),
            ))
        }
    }

    fn read(&self, name: &str) -> marionette_platform::Result<Option<Vec<u8>>> {
        marionette_platform::check_name(name)?;
        let s = self.steam.inner();
        let c = cstr(name);
        unsafe {
            if !(s.f.SteamAPI_ISteamRemoteStorage_FileExists)(s.i.storage, c.as_ptr()) {
                return Ok(None);
            }
            let size = (s.f.SteamAPI_ISteamRemoteStorage_GetFileSize)(s.i.storage, c.as_ptr()).max(0);
            let mut buf = vec![0u8; size as usize];
            let n = (s.f.SteamAPI_ISteamRemoteStorage_FileRead)(
                s.i.storage,
                c.as_ptr(),
                buf.as_mut_ptr().cast(),
                size,
            );
            buf.truncate(n.max(0) as usize);
            Ok(Some(buf))
        }
    }

    fn delete(&mut self, name: &str) -> marionette_platform::Result<()> {
        marionette_platform::check_name(name)?;
        let s = self.steam.inner();
        let c = cstr(name);
        unsafe { (s.f.SteamAPI_ISteamRemoteStorage_FileDelete)(s.i.storage, c.as_ptr()) };
        Ok(())
    }

    fn list(&self) -> marionette_platform::Result<Vec<(String, u64)>> {
        let s = self.steam.inner();
        let mut out = Vec::new();
        unsafe {
            for i in 0..(s.f.SteamAPI_ISteamRemoteStorage_GetFileCount)(s.i.storage) {
                let mut size = 0i32;
                let p = (s.f.SteamAPI_ISteamRemoteStorage_GetFileNameAndSize)(s.i.storage, i, &mut size);
                if !p.is_null() {
                    out.push((
                        CStr::from_ptr(p).to_string_lossy().into_owned(),
                        size.max(0) as u64,
                    ));
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_is_a_clean_error() {
        let e = Steam::load(Some(Path::new("no-such-dir/steam_api_missing.lib")))
            .err()
            .unwrap();
        assert_eq!(e.code, codes::LIBRARY);
        assert!(e.message.contains("cannot load"));
    }

    #[test]
    fn unrelated_library_lacks_functions() {
        // Any shared library that is not steam_api: the C runtime is always present.
        let libc = if cfg!(windows) {
            "kernel32.dll"
        } else if cfg!(target_os = "macos") {
            "libSystem.dylib"
        } else {
            "libc.so.6"
        };
        let e = Steam::load(Some(Path::new(libc))).err().unwrap();
        assert_eq!(e.code, codes::LIBRARY);
        assert!(e.message.contains("SteamAPI_"), "{}", e.message);
    }
}
