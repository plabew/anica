// =========================================
// =========================================
// build.rs

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    // Windows GPUI render/layout paths can exceed the default 1MB main-thread stack.
    if target_os == "windows" && target_env == "msvc" {
        println!("cargo:rustc-link-arg-bin=anica=/STACK:8388608");
    } else if target_os == "windows" && target_env == "gnu" {
        println!("cargo:rustc-link-arg-bin=anica=-Wl,--stack,8388608");
    }
}
