//! Prints the port report and every skeleton for a folder: `cargo run -p marionette-port --example scan -- <path>`.

fn main() {
    let path = std::env::args().nth(1).expect("path");
    let a = marionette_port::scan(std::path::Path::new(&path)).unwrap();
    println!("{}", serde_json::to_string_pretty(&a.report(0, 50, 3)).unwrap());
    for (ci, c) in a.classes.iter().enumerate() {
        if marionette_port::Analysis::makes_script(c) {
            println!(
                "===== {}\n{}",
                c.raw.name,
                marionette_port::scaffold::skeleton(&a, ci, "scripts/x.ts")
            );
        }
    }
}
