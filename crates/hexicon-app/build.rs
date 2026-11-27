//! The player has the web page built into it, WebAssembly and all: check
//! that it has been built.

fn main() {
    let web = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web");
    for file in [
        "index.html",
        "main.js",
        "style.css",
        "pkg/hexicon_wasm.js",
        "pkg/hexicon_wasm_bg.wasm",
    ] {
        let path = web.join(file);
        if !path.exists() {
            panic!(
                "the player shows the web page, and web/{file} is not there: \
                 build the page first with tools/build-web.sh (or build the player \
                 with tools/build-app.sh, which does both)"
            );
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }
}
