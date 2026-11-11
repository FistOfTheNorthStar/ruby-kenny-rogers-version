// `kenny_frame_pointers`: the runtime is built with frame pointers on every frame (on Linux
// by .cargo/config.toml), so the collector's precise frames work there too (stackmap.rs
// `precise_enabled`).
fn main() {
    println!("cargo::rustc-check-cfg=cfg(kenny_frame_pointers)");
    println!("cargo::rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    let mut on = false;
    let mut prev_c = false;
    for f in flags.split('\x1f') {
        let opt = if prev_c { Some(f) } else { f.strip_prefix("-C") };
        prev_c = f == "-C";
        if let Some(o) = opt.map(str::trim).filter(|o| o.starts_with("force-frame-pointers")) {
            on = matches!(o.split('=').nth(1).unwrap_or("yes"), "yes" | "y" | "on" | "true");
        }
    }
    if on {
        println!("cargo::rustc-cfg=kenny_frame_pointers");
    }
}
