fn main() {
    println!(
        "cargo:rustc-env=VINYL_TARGET={}",
        std::env::var("TARGET").unwrap()
    );
    tauri_build::build();
}
