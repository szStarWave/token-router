// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "ota" {
        std::process::exit(token_router_desktop_lib::ota::run_ota_apply_cli(&args[1..]));
    }
    token_router_desktop_lib::run();
}
