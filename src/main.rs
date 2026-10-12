#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
mod app;
#[cfg(feature = "ui-smoke")]
mod ui_smoke;
#[cfg(feature = "ui-smoke")]
mod ui_stage2_smoke;
#[cfg(feature = "ui-smoke")]
mod ui_stage3_smoke;
#[cfg(feature = "ui-smoke")]
mod ui_stage4_smoke;
slint::include_modules!();

fn main() {
    if let Err(error) = app::run() {
        rfd::MessageDialog::new()
            .set_title("Minecraft Explorer")
            .set_description(format!("无法启动：{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}
