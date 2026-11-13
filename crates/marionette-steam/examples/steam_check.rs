//! Checks that Steam works for a game on this machine: loads the steam_api library, starts
//! the API, prints the app id and whether cloud files are on. Needs the Steam client running
//! and `steam_appid.txt` in the working folder (app id 480 is Valve's public test app).
//!
//! `cargo run -p marionette-steam --example steam_check -- [path/to/steam_api64.dll]`

use marionette_platform::CloudFiles;

fn main() {
    let lib = std::env::args().nth(1).map(std::path::PathBuf::from);
    match marionette_steam::Steam::load(lib.as_deref()) {
        Ok((steam, info)) => {
            steam.run_callbacks();
            let cloud = steam.platform().enabled();
            println!(
                "{{\"ok\":true,\"library\":{:?},\"app_id\":{},\"cloud\":{cloud}}}",
                info.library.display().to_string(),
                info.app_id
            );
        }
        Err(e) => {
            println!(
                "{{\"ok\":false,\"code\":{:?},\"message\":{:?}}}",
                e.code, e.message
            );
            std::process::exit(1);
        }
    }
}
