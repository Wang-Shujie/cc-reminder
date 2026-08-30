// GUI subsystem in release builds: without this the packaged app runs as a
// console program and a terminal window rides along for its whole lifetime.
// Debug builds keep the console so logs are visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    cc_reminder_lib::run();
}
