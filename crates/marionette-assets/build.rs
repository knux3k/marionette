//! `intel_tex_2` (feature `compress`, decision 0283) bundles a C++ object that needs the C++
//! runtime when linking on Linux; MSVC and macOS link it on their own. Only BC7 (plain ISPC
//! code) is called.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let compress = std::env::var_os("CARGO_FEATURE_COMPRESS").is_some();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if compress && (os == "linux" || os == "android") {
        println!("cargo:rustc-link-lib=dylib=stdc++");
    }
}
