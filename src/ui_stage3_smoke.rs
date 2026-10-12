//! 显式启用的原生界面文件操作验收；所有源与目标均
//! 验证位于测试器标记的生成目录内部。
use crate::{
    AppWindow,
    ui_stage2_smoke::{native_select_first, snapshot},
};
use anyhow::{Context, Result, ensure};
use slint::{ComponentHandle, Model, Timer, TimerMode};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Run {
    output: PathBuf,
    a: PathBuf,
    b: PathBuf,
    backup: PathBuf,
    stage: u32,
    started: Instant,
    checks: Vec<String>,
}
impl Run {
    fn step(&mut self, ui: &AppWindow) -> Result<bool> {
        let old = self.a.join("saves/原世界");
        let new = self.a.join("saves/新世界");
        ensure!(
            self.started.elapsed() < Duration::from_secs(120),
            "stage {} timed out: {} / {}",
            self.stage,
            ui.get_status(),
            ui.get_operation_preview()
        );
        if ui.get_scanning() {
            return Ok(false);
        }
        if [4, 6, 8, 10].contains(&self.stage) && ui.get_operation_visible() {
            ensure!(
                ui.get_operation_preview().contains("正在执行"),
                "file-operation UI reported a failure: {}",
                ui.get_operation_preview()
            );
        }
        match self.stage {
            0 => {
                if ui.get_entries().row_count() != 1 {
                    return Ok(false);
                }
                native_select_first(ui)?;
                ui.set_detail_favorite(true);
                ui.set_detail_tags("阶段三标签".into());
                ui.invoke_annotate_selected();
                self.checks
                    .push("generated world selected by native pointer event".into());
            }
            1 => {
                if !ui.get_entries().row_data(0).unwrap().title.starts_with("★") {
                    return Ok(false);
                }
                ui.invoke_begin_operation(0, true);
                ui.set_operation_name("新世界".into());
                ui.invoke_prepare_operation();
            }
            2 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ensure!(
                    ui.get_operation_preview().contains("原世界")
                        && ui.get_operation_preview().contains("新世界")
                );
                ui.invoke_confirm_operation();
                ensure!(
                    old.exists() && !new.exists(),
                    "confirmation bypassed explicit checkbox"
                );
                ui.set_operation_name("待修改".into());
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
                ensure!(
                    !ui.get_operation_ready() && old.exists() && !new.exists(),
                    "edited arguments retained old confirmation"
                );
                ui.set_operation_name("新世界".into());
                ui.invoke_prepare_operation();
                self.checks.push("preview contains physical paths; unchecked confirmation and stale edits cannot execute".into());
            }
            3 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                snapshot(ui, &self.output, "rename-confirmation.png")?;
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
            }
            4 => {
                if ui.get_operation_visible() || old.exists() || !new.exists() {
                    return Ok(false);
                }
                let row = ui
                    .get_entries()
                    .row_data(0)
                    .context("renamed row missing")?;
                if !row.path.contains("新世界") {
                    return Ok(false);
                }
                ensure!(row.title.starts_with("★"), "favorite lost on rename");
                native_select_first(ui)?;
                ensure!(ui.get_detail_tags().contains("阶段三标签"));
                ui.invoke_begin_operation(1, true);
                ui.set_operation_parent(self.b.join("saves").to_string_lossy().into_owned().into());
                ui.invoke_prepare_operation();
                self.checks
                    .push("rename synchronizes filesystem, index, favorite and tag".into());
            }
            5 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
            }
            6 => {
                if ui.get_operation_visible() || ui.get_entries().row_count() != 2 {
                    return Ok(false);
                }
                ensure!(new.exists() && self.b.join("saves/新世界/level.dat").exists());
                ui.invoke_select_all();
                ensure!(ui.get_checked_count() == 2);
                ensure!(
                    (0..ui.get_entries().row_count()).all(|n| ui
                        .get_entries()
                        .row_data(n)
                        .unwrap()
                        .checked)
                );
                ui.invoke_begin_operation(3, false);
                ui.set_operation_parent(self.backup.to_string_lossy().into_owned().into());
                ui.set_operation_conflict(1);
                ui.invoke_prepare_operation();
                self.checks.push("copy preserves source and adds target; multiple selection reaches operation preview".into());
            }
            7 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ensure!(ui.get_operation_preview().contains("2 个条目"));
                snapshot(ui, &self.output, "batch-backup.png")?;
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
            }
            8 => {
                if ui.get_operation_visible() {
                    return Ok(false);
                }
                let backups = fs::read_dir(&self.backup)?
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.join("minecraft-explorer-backup.json").is_file())
                    .collect::<Vec<_>>();
                if backups.len() != 2 {
                    return Ok(false);
                }
                ui.invoke_clear_selection();
                ensure!(ui.get_checked_count() == 0);
                ui.invoke_begin_operation(5, false);
                ui.set_operation_backup(backups[0].to_string_lossy().into_owned().into());
                ui.set_operation_parent(self.a.join("saves").to_string_lossy().into_owned().into());
                ui.set_operation_conflict(1);
                ui.invoke_prepare_operation();
                self.checks.push(
                    "batch directory backups contain receipts and distinct collision-safe names"
                        .into(),
                );
            }
            9 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ensure!(ui.get_operation_preview().contains("新世界 (1)"));
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
            }
            10 => {
                if ui.get_operation_visible() || ui.get_entries().row_count() != 3 {
                    return Ok(false);
                }
                ensure!(self.a.join("saves/新世界 (1)/level.dat").exists());
                ui.set_page(7);
                ui.invoke_navigate(7);
                ui.invoke_show_history();
                self.checks
                    .push("restore verifies backup and avoids overwriting existing world".into());
            }
            11 => {
                if !ui.get_operation_history().contains("恢复备份") {
                    return Ok(false);
                }
                ensure!(
                    ui.get_operation_history().contains("重命名")
                        && ui.get_operation_history().contains("复制")
                        && ui.get_operation_history().contains("备份")
                );
                snapshot(ui, &self.output, "operation-history.png")?;
                ui.set_page(2);
                ui.invoke_navigate(2);
                self.checks.push(
                    "operation history exposes completed paths after filesystem changes".into(),
                );
            }
            12 => {
                if ui.get_entries().row_count() != 3 {
                    return Ok(false);
                }
                native_select_first(ui)?;
                ui.invoke_begin_operation(4, true);
                ui.invoke_prepare_operation();
            }
            13 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ensure!(ui.get_operation_preview().contains("Windows 回收站"));
                snapshot(ui, &self.output, "recycle-preview.png")?;
                ui.invoke_dismiss_operation();
                ensure!(!ui.get_operation_visible());
                ensure!(new.exists());
                self.checks
                    .push("recycle preview is explicit; dismissing performs no deletion".into());
                ui.invoke_begin_operation(1, true);
                ui.set_operation_parent(self.b.join("saves").to_string_lossy().into_owned().into());
                ui.set_operation_conflict(1);
                ui.invoke_prepare_operation();
            }
            14 => {
                if !ui.get_operation_ready() {
                    return Ok(false);
                }
                ui.set_operation_confirmed(true);
                ui.invoke_confirm_operation();
                ensure!(ui.get_scanning(), "queued write did not mark UI busy");
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::CloseRequested);
                ensure!(
                    ui.get_status().contains("收尾后关闭"),
                    "busy close did not request cleanup before exiting"
                );
                self.checks.push(
                    "closing a queued file operation requests cancellation and waits for cleanup"
                        .into(),
                );
                fs::write(
                    self.output.join("ui-result.json"),
                    serde_json::to_vec_pretty(
                        &serde_json::json!({"passed":true,"checks":self.checks,"elapsed_seconds":self.started.elapsed().as_secs_f64(),"real_game_files_modified":false}),
                    )?,
                )?;
                // 必须由应用的关闭处理器完成任务收尾
                // 并退出原生事件循环。
            }
            _ => return Ok(false),
        }
        self.stage += 1;
        Ok(false)
    }
}
pub fn start(ui: &AppWindow) -> Option<Timer> {
    let output = PathBuf::from(std::env::var_os("MINECRAFT_EXPLORER_UI_STAGE3")?);
    let output = fs::canonicalize(output).expect("stage 3 output directory");
    assert_eq!(
        fs::read(output.join("stage3-fixture.marker")).unwrap(),
        b"minecraft-explorer-generated-stage3-fixture"
    );
    let a = fs::canonicalize(output.join("Minecraft 甲")).unwrap();
    let b = fs::canonicalize(output.join("Minecraft 乙")).unwrap();
    let backup = fs::canonicalize(output.join("备份")).unwrap();
    assert!(a.starts_with(&output) && b.starts_with(&output) && backup.starts_with(&output));
    let mut run = Run {
        output,
        a,
        b,
        backup,
        stage: 0,
        started: Instant::now(),
        checks: Vec::new(),
    };
    let weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(200), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match run.step(&ui) {
            Ok(true) => {
                slint::quit_event_loop().unwrap();
            }
            Ok(false) => (),
            Err(e) => {
                fs::write(
                    run.output.join("ui-result.json"),
                    serde_json::json!({"passed":false,"stage":run.stage,"error":format!("{e:#}")})
                        .to_string(),
                )
                .unwrap();
                slint::quit_event_loop().unwrap();
            }
        }
    });
    Some(timer)
}
