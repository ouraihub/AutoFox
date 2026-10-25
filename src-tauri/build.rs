//! Tauri build script.

fn main() {
    // `ipc.rs` embeds `client.ts` with `include_str!` to assert the ErrorCode
    // union covers every Rust variant. Cargo does not infer that dependency,
    // so without this an edit to `client.ts` alone would re-run the test
    // against stale embedded content and pass on real drift.
    println!("cargo:rerun-if-changed=../src/lib/ipc/client.ts");
    tauri_build::build();
}
