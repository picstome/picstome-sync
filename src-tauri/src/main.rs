#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let smoke = std::env::args().any(|arg| arg == "--smoke")
        || std::env::var("PICSTOME_SMOKE")
            .map(|value| value == "1")
            .unwrap_or(false);
    if smoke {
        std::process::exit(picstome_sync_lib::smoke::run());
    }
    picstome_sync_lib::run();
}
