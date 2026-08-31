//! 原生托盘(v2-issues,设计 §18.3):常驻图标 + 菜单——打开 / 健康
//! 状态 / 暂停×3 / 恢复 / 退出。菜单动作调用与命令层相同的 typed impl;
//! 健康条目随 `core://health-changed` 重建刷新。左侧单击 = 打开主窗。

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::commands::settings::PauseDurationInput;
use crate::commands::{CoreState, health_snapshot};
use crate::health::HealthLevel;
use crate::worker::{CoreEvent, emit};

const TRAY_ID: &str = "main-tray";
const ID_OPEN: &str = "tray-open";
const ID_PAUSE_15: &str = "tray-pause-15";
const ID_PAUSE_60: &str = "tray-pause-60";
const ID_PAUSE_TODAY: &str = "tray-pause-today";
const ID_RESUME: &str = "tray-resume";
const ID_QUIT: &str = "tray-quit";

/// zh-CN 是权威字典,en 精确镜像——与前端 i18n 同规则。
struct Labels {
    open: &'static str,
    health: [&'static str; 3], // [ok, warning, error]
    pause15: &'static str,
    pause60: &'static str,
    pause_today: &'static str,
    resume: &'static str,
    quit: &'static str,
}

fn labels_for(locale: &str) -> Labels {
    if locale == "en" {
        Labels {
            open: "Open CC Reminder",
            health: ["Health: OK", "Health: attention", "Health: fault"],
            pause15: "Pause notifications 15 min",
            pause60: "Pause notifications 1 hour",
            pause_today: "Pause until end of day",
            resume: "Resume notifications",
            quit: "Quit",
        }
    } else {
        Labels {
            open: "打开 CC Reminder",
            health: ["健康:正常", "健康:需要注意", "健康:存在异常"],
            pause15: "暂停通知 15 分钟",
            pause60: "暂停通知 1 小时",
            pause_today: "暂停到今天结束",
            resume: "恢复通知",
            quit: "退出",
        }
    }
}

fn health_text(labels: &Labels, level: HealthLevel) -> String {
    let index = match level {
        HealthLevel::Ok => 0,
        HealthLevel::Warning => 1,
        HealthLevel::Error => 2,
    };
    labels.health[index].to_owned()
}

fn build_menu(
    app: &AppHandle,
    labels: &Labels,
    level: HealthLevel,
) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, ID_OPEN, labels.open, true, None::<&str>)?;
    let health = MenuItem::with_id(
        app,
        "tray-health",
        health_text(labels, level),
        false,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let pause15 = MenuItem::with_id(app, ID_PAUSE_15, labels.pause15, true, None::<&str>)?;
    let pause60 = MenuItem::with_id(app, ID_PAUSE_60, labels.pause60, true, None::<&str>)?;
    let pause_today =
        MenuItem::with_id(app, ID_PAUSE_TODAY, labels.pause_today, true, None::<&str>)?;
    let resume = MenuItem::with_id(app, ID_RESUME, labels.resume, true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, ID_QUIT, labels.quit, true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &open,
            &health,
            &sep1,
            &pause15,
            &pause60,
            &pause_today,
            &resume,
            &sep2,
            &quit,
        ],
    )
}

fn locale_of(app: &AppHandle) -> String {
    app.try_state::<CoreState>()
        .and_then(|state| state.storage.config.get_settings().ok())
        .map(|settings| match settings.locale {
            crate::model::Locale::En => "en".to_owned(),
            crate::model::Locale::ZhCn => "zh_cn".to_owned(),
        })
        .unwrap_or_else(|| "zh_cn".to_owned())
}

fn show_main(app: &AppHandle) {
    // 托盘"打开"/左键点击是关闭进托盘后的主要恢复入口:窗口回来的同时
    // 把隐藏的 macOS Dock 图标带回来(见 lib.rs 的 CloseRequested 隐藏点)。
    crate::set_dock_visible(app, true);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn handle_menu_action(app: &AppHandle, id: &str) {
    match id {
        ID_OPEN => show_main(app),
        ID_PAUSE_15 => apply_pause(app, PauseDurationInput::FifteenMinutes),
        ID_PAUSE_60 => apply_pause(app, PauseDurationInput::OneHour),
        ID_PAUSE_TODAY => apply_pause(app, PauseDurationInput::Today),
        ID_RESUME => run_state_action(app, |state| {
            crate::commands::settings::clear_notification_pause_impl(state)
        }),
        ID_QUIT => app.exit(0),
        _ => {}
    }
}

fn apply_pause(app: &AppHandle, duration: PauseDurationInput) {
    run_state_action(app, move |state| {
        let now = chrono::Local::now().fixed_offset();
        crate::commands::settings::set_notification_pause_impl(state, duration, now)
    });
}

/// 菜单动作与命令层同源:同一 typed impl、同一 health-changed 广播,
/// 设置页与托盘永远一致。
fn run_state_action<F>(app: &AppHandle, action: F)
where
    F: FnOnce(
            &CoreState,
        ) -> Result<crate::commands::settings::SettingsView, crate::error::AppError>
        + Send
        + 'static,
{
    if let Some(state) = app.try_state::<CoreState>() {
        let state = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || {
            let _ = action(&state);
            emit(
                &state.runtime.core_events,
                CoreEvent::HealthChanged { channel_id: None },
            );
        });
    }
}

/// 安装托盘(幂等:重复调用仅刷新菜单)。在 CoreState manage 之后调用。
pub fn install(app: &AppHandle) {
    let locale = locale_of(app);
    let labels = labels_for(&locale);
    let level = current_level(app);
    let existing = app.tray_by_id(TRAY_ID);
    match build_menu(app, &labels, level) {
        Ok(menu) => match existing {
            Some(tray) => {
                let _ = tray.set_menu(Some(menu));
            }
            None => {
                let mut builder = TrayIconBuilder::with_id(TRAY_ID)
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| handle_menu_action(app, event.id().as_ref()))
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main(tray.app_handle());
                        }
                    });
                if let Some(icon) = app.default_window_icon().cloned() {
                    builder = builder.icon(icon);
                }
                if let Err(error) = builder.build(app) {
                    app.try_state::<CoreState>().inspect(|state| {
                        state
                            .diagnostics
                            .info("tray", &format!("tray icon build failed: {error}"));
                    });
                }
            }
        },
        Err(error) => {
            app.try_state::<CoreState>().inspect(|state| {
                state
                    .diagnostics
                    .info("tray", &format!("tray menu build failed: {error}"));
            });
        }
    }
}

fn current_level(app: &AppHandle) -> HealthLevel {
    app.try_state::<CoreState>()
        .and_then(|state| health_snapshot(state.inner()).ok())
        .map(|snapshot| snapshot.overall)
        .unwrap_or(HealthLevel::Ok)
}

/// health-changed 后由 forwarder 调用。**必须在主线程动 AppKit**:菜单/
/// 托盘是 NSMenu/NSStatusItem,off-main-thread 变更是未定义行为(实机
/// 教训 2026-08-27:06:48 重检触发的一次 off-main 重建与上午 IPC 环失联
/// 及两次退出 abort 同时段)。经 run_on_main_thread 派发,重复调用安全。
pub fn refresh_health(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || install(&handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_text_maps_levels_in_both_locales() {
        let zh = labels_for("zh_cn");
        assert_eq!(health_text(&zh, HealthLevel::Ok), "健康:正常");
        assert_eq!(health_text(&zh, HealthLevel::Warning), "健康:需要注意");
        assert_eq!(health_text(&zh, HealthLevel::Error), "健康:存在异常");
        let en = labels_for("en");
        assert_eq!(health_text(&en, HealthLevel::Error), "Health: fault");
        assert_eq!(health_text(&en, HealthLevel::Ok), "Health: OK");
    }

    #[test]
    fn unknown_locale_defaults_to_authoritative_zh() {
        let labels = labels_for("klingon");
        assert_eq!(labels.open, "打开 CC Reminder");
    }
}
