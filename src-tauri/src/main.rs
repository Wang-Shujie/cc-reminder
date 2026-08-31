// GUI subsystem in release builds: without this the packaged app runs as a
// console program and a terminal window rides along for its whole lifetime.
// Debug builds keep the console so logs are visible during development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // CLI 子命令（NSIS 卸载器的 PREUNINSTALL 钩子调用，无 GUI）：
    //   cc-reminder.exe --uninstall-hooks
    // 从 ~/.claude/settings.json 与 ~/.codex/hooks.json 精准移除 CC Reminder
    // 的自有条目（事务与 GUI 内的卸载完全一致）。任何其他调用走 GUI。
    if std::env::args().any(|arg| arg == "--uninstall-hooks") {
        std::process::exit(cc_reminder_lib::uninstall_hooks_cli());
    }
    cc_reminder_lib::run();
}
