// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = bull_arrives_lib::agent::launcher_exit_code() {
        std::process::exit(code);
    }
    bull_arrives_lib::run()
}
