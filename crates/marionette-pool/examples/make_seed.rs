//! Writes the seed set (assets made by `src/seed.rs`) into a pool folder.
//! `cargo run -p marionette-pool --example make_seed -- pool`

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "pool".into());
    let pool = marionette_pool::Pool::at(std::path::Path::new(&dir), marionette_pool::PoolFrom::Input)
        .expect("pool location");
    match marionette_pool::seed::build(&pool) {
        Ok(reports) => {
            for r in reports {
                println!("{} {}", r.status, r.id);
            }
            let index = pool.index_text();
            marionette_pool::write_file(&pool.root().join("index.json"), index.as_bytes()).expect("index");
        }
        Err(e) => {
            eprintln!("{}", serde_json::to_string_pretty(&e.0).expect("json"));
            std::process::exit(1);
        }
    }
}
