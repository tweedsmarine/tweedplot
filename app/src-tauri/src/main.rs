// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    racenav_app_lib::migrate_from_racenav();
    racenav_app_lib::run()
}
