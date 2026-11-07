//! On Windows, SDL2.dll comes from the setup program, not with the .exe. Loading it
//! late lets the app check for it first and explain what to do, instead of Windows
//! refusing to start the app with an error about a missing DLL.

fn main() {
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows_msvc {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:SDL2.dll");
        println!("cargo:rustc-link-arg-bins=delayimp.lib");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
