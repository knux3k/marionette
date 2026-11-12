//! Converts one FBX file to GLB: `cargo run -p marionette-import --example fbx2glb -- in.fbx out.glb`.
//! Textures are looked up next to the FBX file by their relative name.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = args.as_slice() else {
        eprintln!("usage: fbx2glb <in.fbx> <out.glb>");
        std::process::exit(2);
    };
    let bytes = std::fs::read(input).expect("read input");
    let dir = std::path::Path::new(input).parent().unwrap().to_path_buf();
    let mut resolve = |r: &marionette_import::fbx::TextureRequest| {
        if let Some(b) = &r.embedded {
            return Ok(marionette_import::fbx::ResolvedImage {
                bytes: b.clone(),
                mime: "image/png",
                origin: "embedded".into(),
            });
        }
        std::fs::read(dir.join(r.file_name()))
            .map(|b| marionette_import::fbx::ResolvedImage {
                bytes: b,
                mime: "image/png",
                origin: "beside the file".into(),
            })
            .map_err(|e| e.to_string())
    };
    let c = marionette_import::fbx::convert(&bytes, Default::default(), &mut resolve).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    std::fs::write(output, &c.glb).expect("write output");
    println!("{}", serde_json::to_string_pretty(&c.summary).unwrap());
    for n in c.notes {
        println!("note: {n}");
    }
}
