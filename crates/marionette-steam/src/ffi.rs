//! The few Steamworks flat-API entry points this backend calls, resolved by name from the
//! steam_api library the game ships. Declarations follow the public C ABI of that library;
//! no SDK header or binary is part of this repository (decision 0212).

use libloading::Library;
use std::ffi::{c_char, c_int, c_void};

pub type Ptr = *mut c_void;

/// Callback ids (`k_iCallback`) and other numbers from the flat API.
pub mod id {
    pub const API_CALL_COMPLETED: i32 = 703;
    pub const LOBBY_ENTER: i32 = 504;
    pub const LOBBY_DATA_UPDATE: i32 = 505;
    pub const LOBBY_CHAT_UPDATE: i32 = 506;
    pub const LOBBY_MATCH_LIST: i32 = 510;
    pub const LOBBY_CREATED: i32 = 513;
    pub const NET_SESSION_REQUEST: i32 = 1251;
    pub const SEND_UNRELIABLE_NO_NAGLE: i32 = 1;
    pub const SEND_RELIABLE: i32 = 8;
    pub const SEND_AUTO_RESTART: i32 = 32;
    pub const RESULT_OK: i32 = 1;
    pub const CHAT_ENTER_SUCCESS: u32 = 1;
    pub const LOBBY_PRIVATE: i32 = 0;
    pub const LOBBY_FRIENDS_ONLY: i32 = 1;
    pub const LOBBY_PUBLIC: i32 = 2;
    pub const COMPARE_EQUAL: i32 = 0;
    pub const IDENTITY_SIZE: usize = 136;
    /// Largest reliable message the library takes in one send.
    pub const MAX_MESSAGE: usize = 512 * 1024;
}

/// `CallbackMsg_t`: packed to 4 bytes except on Windows (natural alignment).
#[cfg_attr(not(windows), repr(C, packed(4)))]
#[cfg_attr(windows, repr(C))]
#[derive(Clone, Copy)]
pub struct CallbackMsg {
    pub user: i32,
    pub callback: c_int,
    pub param: *mut u8,
    pub param_size: c_int,
}

/// Byte offsets inside callback structs that differ by packing.
pub mod offset {
    /// `LobbyCreated_t::m_ulSteamIDLobby` (after a 4-byte result).
    pub const LOBBY_CREATED_ID: usize = if cfg!(windows) { 8 } else { 4 };
    /// `SteamNetworkingMessage_t`: data pointer, size, sender identity.
    pub const MSG_DATA: usize = 0;
    pub const MSG_SIZE: usize = 8;
    pub const MSG_PEER: usize = 16;
}

macro_rules! fns {
    ($($name:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        pub struct Fns { $(pub $name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)* }
        impl Fns {
            /// Resolves every required symbol; the error names the first missing one.
            pub unsafe fn load(lib: &Library) -> Result<Self, String> {
                unsafe {
                    Ok(Self { $($name: *lib
                        .get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                        .map_err(|e| format!("{}: {e}", stringify!($name)))?,)* })
                }
            }
        }
    };
}

fns! {
    SteamAPI_Shutdown: fn();
    SteamAPI_IsSteamRunning: fn() -> bool;
    SteamAPI_GetHSteamPipe: fn() -> i32;
    SteamAPI_ManualDispatch_Init: fn();
    SteamAPI_ManualDispatch_RunFrame: fn(i32);
    SteamAPI_ManualDispatch_GetNextCallback: fn(i32, *mut CallbackMsg) -> bool;
    SteamAPI_ManualDispatch_FreeLastCallback: fn(i32);
    SteamAPI_ManualDispatch_GetAPICallResult: fn(i32, u64, Ptr, c_int, c_int, *mut bool) -> bool;
    SteamAPI_ISteamUser_GetSteamID: fn(Ptr) -> u64;
    SteamAPI_ISteamUtils_GetAppID: fn(Ptr) -> u32;
    SteamAPI_SteamNetworkingIdentity_SetSteamID64: fn(Ptr, u64);
    SteamAPI_SteamNetworkingIdentity_GetSteamID64: fn(Ptr) -> u64;
    SteamAPI_ISteamNetworkingMessages_SendMessageToUser: fn(Ptr, *const c_void, *const c_void, u32, c_int, c_int) -> c_int;
    SteamAPI_ISteamNetworkingMessages_ReceiveMessagesOnChannel: fn(Ptr, c_int, *mut Ptr, c_int) -> c_int;
    SteamAPI_ISteamNetworkingMessages_AcceptSessionWithUser: fn(Ptr, *const c_void) -> bool;
    SteamAPI_SteamNetworkingMessage_t_Release: fn(Ptr);
    SteamAPI_ISteamNetworkingUtils_InitRelayNetworkAccess: fn(Ptr);
    SteamAPI_ISteamMatchmaking_CreateLobby: fn(Ptr, c_int, c_int) -> u64;
    SteamAPI_ISteamMatchmaking_JoinLobby: fn(Ptr, u64) -> u64;
    SteamAPI_ISteamMatchmaking_LeaveLobby: fn(Ptr, u64);
    SteamAPI_ISteamMatchmaking_RequestLobbyList: fn(Ptr) -> u64;
    SteamAPI_ISteamMatchmaking_AddRequestLobbyListStringFilter: fn(Ptr, *const c_char, *const c_char, c_int);
    SteamAPI_ISteamMatchmaking_GetLobbyByIndex: fn(Ptr, c_int) -> u64;
    SteamAPI_ISteamMatchmaking_GetNumLobbyMembers: fn(Ptr, u64) -> c_int;
    SteamAPI_ISteamMatchmaking_GetLobbyMemberByIndex: fn(Ptr, u64, c_int) -> u64;
    SteamAPI_ISteamMatchmaking_GetLobbyOwner: fn(Ptr, u64) -> u64;
    SteamAPI_ISteamMatchmaking_GetLobbyMemberLimit: fn(Ptr, u64) -> c_int;
    SteamAPI_ISteamMatchmaking_SetLobbyData: fn(Ptr, u64, *const c_char, *const c_char) -> bool;
    SteamAPI_ISteamMatchmaking_GetLobbyDataCount: fn(Ptr, u64) -> c_int;
    SteamAPI_ISteamMatchmaking_GetLobbyDataByIndex: fn(Ptr, u64, c_int, *mut c_char, c_int, *mut c_char, c_int) -> bool;
    SteamAPI_ISteamUserStats_GetAchievement: fn(Ptr, *const c_char, *mut bool) -> bool;
    SteamAPI_ISteamUserStats_SetAchievement: fn(Ptr, *const c_char) -> bool;
    SteamAPI_ISteamUserStats_ClearAchievement: fn(Ptr, *const c_char) -> bool;
    SteamAPI_ISteamUserStats_IndicateAchievementProgress: fn(Ptr, *const c_char, u32, u32) -> bool;
    SteamAPI_ISteamUserStats_GetStatInt32: fn(Ptr, *const c_char, *mut i32) -> bool;
    SteamAPI_ISteamUserStats_GetStatFloat: fn(Ptr, *const c_char, *mut f32) -> bool;
    SteamAPI_ISteamUserStats_SetStatInt32: fn(Ptr, *const c_char, i32) -> bool;
    SteamAPI_ISteamUserStats_SetStatFloat: fn(Ptr, *const c_char, f32) -> bool;
    SteamAPI_ISteamUserStats_StoreStats: fn(Ptr) -> bool;
    SteamAPI_ISteamRemoteStorage_IsCloudEnabledForAccount: fn(Ptr) -> bool;
    SteamAPI_ISteamRemoteStorage_IsCloudEnabledForApp: fn(Ptr) -> bool;
    SteamAPI_ISteamRemoteStorage_FileWrite: fn(Ptr, *const c_char, *const c_void, i32) -> bool;
    SteamAPI_ISteamRemoteStorage_FileRead: fn(Ptr, *const c_char, Ptr, i32) -> i32;
    SteamAPI_ISteamRemoteStorage_FileExists: fn(Ptr, *const c_char) -> bool;
    SteamAPI_ISteamRemoteStorage_GetFileSize: fn(Ptr, *const c_char) -> i32;
    SteamAPI_ISteamRemoteStorage_FileDelete: fn(Ptr, *const c_char) -> bool;
    SteamAPI_ISteamRemoteStorage_GetFileCount: fn(Ptr) -> i32;
    SteamAPI_ISteamRemoteStorage_GetFileNameAndSize: fn(Ptr, c_int, *mut i32) -> *const c_char;
}

/// Interface accessors are versioned; newer libraries first. The first one found is used.
pub const USER: &[&str] = &[
    "SteamAPI_SteamUser_v023",
    "SteamAPI_SteamUser_v022",
    "SteamAPI_SteamUser_v021",
];
pub const UTILS: &[&str] = &["SteamAPI_SteamUtils_v010", "SteamAPI_SteamUtils_v009"];
pub const MATCHMAKING: &[&str] = &["SteamAPI_SteamMatchmaking_v009"];
pub const USER_STATS: &[&str] = &["SteamAPI_SteamUserStats_v013", "SteamAPI_SteamUserStats_v012"];
pub const REMOTE_STORAGE: &[&str] = &[
    "SteamAPI_SteamRemoteStorage_v016",
    "SteamAPI_SteamRemoteStorage_v014",
];
pub const NET_MESSAGES: &[&str] = &["SteamAPI_SteamNetworkingMessages_SteamAPI_v002"];
pub const NET_UTILS: &[&str] = &[
    "SteamAPI_SteamNetworkingUtils_SteamAPI_v004",
    "SteamAPI_SteamNetworkingUtils_SteamAPI_v003",
];

/// Calls the first accessor in `names` that the library exports.
pub unsafe fn interface(lib: &Library, names: &[&str]) -> Result<Ptr, String> {
    for n in names {
        let sym = format!("{n}\0");
        if let Ok(f) = unsafe { lib.get::<unsafe extern "C" fn() -> Ptr>(sym.as_bytes()) } {
            let p = unsafe { f() };
            if !p.is_null() {
                return Ok(p);
            }
        }
    }
    Err(format!("none of {} found or initialised", names.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_the_flat_api() {
        let want = if cfg!(windows) { 24 } else { 20 };
        if cfg!(target_pointer_width = "64") {
            assert_eq!(std::mem::size_of::<CallbackMsg>(), want);
        }
    }
}
