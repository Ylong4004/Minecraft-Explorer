//! 阶段 4 原生界面验收，仅操作带标记的生成目录与当前 Slint 窗口。
use crate::{
    AppWindow,
    ui_stage2_smoke::{native_select_first, snapshot},
};
use anyhow::{Result, ensure};
use minecraft_explorer::index::Index;
use slint::{ComponentHandle, Model, Timer, TimerMode, platform::WindowEvent};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(serde::Deserialize)]
struct Fixture {
    root: PathBuf,
    cache: PathBuf,
    bytes: u64,
    extra_root: PathBuf,
    cancel_root: PathBuf,
    #[serde(default)]
    auto_only: bool,
}

/// 原生验收仅替换目录选择器结果，后续校验、保存、扫描和分析仍走正式流程。
pub fn chosen_source() -> Option<PathBuf> {
    let output = fs::canonicalize(PathBuf::from(std::env::var_os(
        "MINECRAFT_EXPLORER_UI_STAGE4",
    )?))
    .ok()?;
    if fs::read(output.join("stage4-fixture.marker"))
        .ok()?
        .as_slice()
        != b"minecraft-explorer-generated-stage4-fixture"
    {
        return None;
    }
    let fixture: Fixture =
        serde_json::from_slice(&fs::read(output.join("fixture.json")).ok()?).ok()?;
    let selected = fs::canonicalize(if output.join("picker-cancel.marker").exists() {
        fixture.cancel_root
    } else {
        fixture.extra_root
    })
    .ok()?;
    selected.starts_with(output).then_some(selected)
}
struct Run {
    output: PathBuf,
    fixture: Fixture,
    stage: usize,
    started: Instant,
    checks: Vec<String>,
    verify: bool,
    linked: bool,
    linked_id: slint::SharedString,
}

fn navigate(ui: &AppWindow, page: i32) {
    ui.set_page(page);
    ui.set_search("".into());
    ui.invoke_navigate(page);
}
fn key(ui: &AppWindow, text: slint::SharedString) {
    ui.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn native_choose_appearance(ui: &AppWindow, english: bool, light: bool) -> Result<()> {
    use slint::{LogicalPosition, platform::PointerEventButton};
    ensure!(ui.get_page() == 8);
    // 设置首行的两个控件使用稳定逻辑布局；向自己的窗口发送真实点击和上下键。
    ui.window().take_snapshot().map_err(anyhow::Error::from)?;
    let width = ui.window().size().width as f32 / ui.window().scale_factor();
    for (fraction, desired, current) in [
        (0.28, english, ui.get_english()),
        (0.78, light, ui.get_light_theme()),
    ] {
        if current == desired {
            continue;
        }
        let position = LogicalPosition::new(204. + (width - 224.) * fraction, 123.);
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
        key(
            ui,
            if desired {
                slint::platform::Key::DownArrow
            } else {
                slint::platform::Key::UpArrow
            }
            .into(),
        );
        key(ui, slint::platform::Key::Return.into());
    }
    ensure!(
        ui.get_english() == english && ui.get_light_theme() == light,
        "真实下拉框选择未改变语言/主题"
    );
    Ok(())
}

fn double_click_tile(ui: &AppWindow, tint: slint::Color) -> Result<()> {
    use slint::{LogicalPosition, platform::PointerEventButton};
    let pixels = ui.window().take_snapshot().map_err(anyhow::Error::from)?;
    let rgb = (tint.red(), tint.green(), tint.blue());
    let mut position = None;
    'search: for y in 140..pixels.height() {
        for x in 210..pixels.width().saturating_sub(300) {
            let p = pixels.as_slice()[(y * pixels.width() + x) as usize];
            if (p.r, p.g, p.b) == rgb {
                let scale = ui.window().scale_factor();
                position = Some(LogicalPosition::new(
                    (x + 6) as f32 / scale,
                    (y + 6) as f32 / scale,
                ));
                break 'search;
            }
        }
    }
    let position = position.ok_or_else(|| anyhow::anyhow!("未找到树图矩形"))?;
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    for _ in 0..2 {
        ui.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        });
        ui.window().dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
    }
    Ok(())
}

impl Run {
    fn auto_only_step(&mut self, ui: &AppWindow) -> Result<bool> {
        if ui.get_scanning() || ui.get_counts().is_empty() {
            return Ok(false);
        }
        if self.verify {
            ensure!(ui.get_english() && ui.get_light_theme());
            ensure!(
                Index::open(&self.fixture.cache.join("index.db"))?
                    .load_settings()?
                    .sources
                    .len()
                    == 1
            );
            self.checks.push("首次添加目录与外观选择重启持久化".into());
            return Ok(true);
        }
        if self.stage == 0 {
            navigate(ui, 8);
            native_choose_appearance(ui, true, true)?;
            ui.invoke_add_source(0);
            self.stage = 1;
        } else if ui.get_space_bytes().as_str() == "73" {
            ensure!(ui.get_page() == 6 && ui.get_space_tiles().row_count() == 1);
            ensure!(
                ui.get_space_scopes()
                    .row_data(ui.get_space_index().max(0) as usize)
                    .unwrap()
                    .contains("自动分析新增目录")
            );
            snapshot(ui, &self.output, "first-small-source-auto-analysis.png")?;
            self.checks
                .push("首次添加小目录的快速扫描/分析结果不被延迟范围回调清空".into());
            return Ok(true);
        }
        Ok(false)
    }

    fn step(&mut self, ui: &AppWindow) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(100),
            "阶段 4 验收超时：{}",
            self.stage
        );
        if self.fixture.auto_only {
            return self.auto_only_step(ui);
        }
        if ui.get_scanning() && self.stage != 13 {
            return Ok(false);
        }
        match self.stage {
            0 => {
                if ui.get_entries().row_count() < 1000 {
                    return Ok(false);
                }
                if self.verify {
                    ensure!(
                        ui.get_english() && ui.get_light_theme(),
                        "语言和主题重启后未保留"
                    );
                    ensure!(ui.get_counts().contains("worlds"), "计数未翻译");
                    snapshot(ui, &self.output, "restart-english.png")?;
                    self.checks
                        .push("语言与主题重启持久化；旧索引继续可用".into());
                    return Ok(true);
                }
                ensure!(!ui.get_english() && !ui.get_light_theme());
                snapshot(ui, &self.output, "worlds-dark-zh.png")?;
                navigate(ui, 6);
            }
            1 => {
                if ui.get_space_scopes().row_count() < 2 {
                    return Ok(false);
                }
                ui.invoke_analyze_space();
            }
            2 => {
                if ui.get_space_tiles().row_count() == 0 {
                    return Ok(false);
                }
                ensure!(
                    ui.get_space_tiles().row_count() >= 3,
                    "单根目录未直接展示类别"
                );
                ensure!(
                    ui.get_space_bytes() == self.fixture.bytes.to_string(),
                    "空间分析总量不等于实际文件总量"
                );
                snapshot(ui, &self.output, "treemap-dark-zh.png")?;
                let saves = ui
                    .get_space_tiles()
                    .iter()
                    .find(|tile| tile.title == "saves")
                    .unwrap();
                double_click_tile(ui, saves.tint)?;
            }
            3 => {
                if !self.linked {
                    ensure!(ui.get_space_can_up());
                    ensure!(ui.get_space_tiles().row_count() >= 3);
                    ensure!(ui.get_space_bars().row_count() == 6);
                    snapshot(ui, &self.output, "treemap-worlds-dark-zh.png")?;
                    ensure!(ui.get_space_tiles().row_count() <= 256, "树图控件没有合并");
                    let world = ui
                        .get_space_tiles()
                        .iter()
                        .find(|tile| !tile.item_id.is_empty())
                        .unwrap();
                    ui.invoke_space_action(world.node, 0);
                    ensure!(ui.get_space_can_view());
                    self.linked_id = world.item_id;
                    ui.invoke_space_action(world.node, 4);
                    self.linked = true;
                    return Ok(false);
                }
                if ui.get_page() != 0
                    || ui.get_entries().row_count() != 1
                    || ui.get_selected_id() != self.linked_id
                {
                    return Ok(false);
                }
                snapshot(ui, &self.output, "treemap-list-link.png")?;
                navigate(ui, 6);
                ui.invoke_space_action(-1, 2);
                self.checks.push(
                    "空间树图、六类统计、共享去重、原生双击钻取、256 矩形上限与列表联动".into(),
                );
                fs::write(self.fixture.root.join("late.bin"), [0; 37])?;
                navigate(ui, 8);
                native_choose_appearance(ui, true, true)?;
                navigate(ui, 6);
            }
            4 => {
                let stored = Index::open(&self.fixture.cache.join("index.db"))?.load_settings()?;
                if !stored.english || !stored.light_theme {
                    return Ok(false);
                }
                ensure!(
                    ui.get_space_summary().starts_with("Logical size"),
                    "空间摘要未翻译"
                );
                ensure!(ui.get_space_advice().contains("Worlds:"), "空间建议未翻译");
                ensure!(ui.get_space_scopes().row_data(0).unwrap() == "All sources");
                ensure!(
                    ui.get_space_bytes() == self.fixture.bytes.to_string(),
                    "切换语言重扫了磁盘"
                );
                snapshot(ui, &self.output, "treemap-light-en.png")?;
                self.checks
                    .push("中英双语即时切换、深浅主题与分析快照复用".into());
                ui.invoke_space_action(-1, 2);
                ui.set_space_index(1);
            }
            5 => {
                ensure!(
                    ui.get_space_tiles().row_count() == 0,
                    "范围变化保留了旧分析"
                );
                ui.invoke_analyze_space();
            }
            6 => {
                if ui.get_space_tiles().row_count() == 0 {
                    return Ok(false);
                }
                ensure!(
                    ui.get_space_bytes() == (self.fixture.bytes + 37).to_string(),
                    "重新分析未读取新增文件"
                );
                ui.invoke_space_action(ui.get_space_tiles().row_data(0).unwrap().node, 1);
                self.checks
                    .push("按根目录分析、切换范围使旧结果失效".into());
                navigate(ui, 2);
            }
            7 => {
                if ui.get_entries().row_count() < 1000 {
                    return Ok(false);
                }
                native_select_first(ui)?;
                ensure!(!ui.get_selected_id().is_empty());
                key(ui, slint::platform::Key::DownArrow.into());
                ensure!(ui.get_current_row_index() == 1, "原生方向键未选中下一项");
                for _ in 0..12 {
                    ui.invoke_keyboard_action(13);
                }
                ui.window().dispatch_event(WindowEvent::KeyPressed {
                    text: slint::platform::Key::Control.into(),
                });
                key(ui, "a".into());
                ensure!(ui.get_checked_count() == 256, "Ctrl+A 未按批量上限勾选");
                ui.window().dispatch_event(WindowEvent::KeyPressed {
                    text: slint::platform::Key::Shift.into(),
                });
                key(ui, "A".into());
                ensure!(ui.get_checked_count() == 0, "Ctrl+Shift+A 未清空勾选");
                ui.window().dispatch_event(WindowEvent::KeyReleased {
                    text: slint::platform::Key::Shift.into(),
                });
                ui.window().dispatch_event(WindowEvent::KeyReleased {
                    text: slint::platform::Key::Control.into(),
                });
                key(ui, slint::platform::Key::F2.into());
                ensure!(
                    ui.get_operation_visible() && ui.get_operation_action() == 0,
                    "F2 未打开重命名预览"
                );
                ui.invoke_dismiss_operation();
                key(ui, slint::platform::Key::Delete.into());
                ensure!(
                    ui.get_operation_visible() && ui.get_operation_action() == 4,
                    "Delete 未打开回收预览"
                );
                ensure!(!ui.get_operation_ready() && !ui.get_operation_confirmed());
                ui.invoke_dismiss_operation();
                self.checks.push(
                    "原生 Ctrl+A/Ctrl+Shift+A、多选上限、方向键滚动；F2/Delete 只打开确认预览"
                        .into(),
                );
                ui.invoke_entry_action(ui.get_selected_id(), 7);
            }
            8 => {
                if !ui.get_detail_favorite() {
                    return Ok(false);
                }
                ensure!(
                    ui.get_detail_text().contains("Game directory"),
                    "详情未翻译"
                );
                snapshot(ui, &self.output, "details-light-en.png")?;
                self.checks
                    .push("右键收藏动作与英文详情、可见行图标、1000 项虚拟列表".into());
                navigate(ui, 8);
            }
            9 => {
                snapshot(ui, &self.output, "settings-light-en.png")?;
                native_choose_appearance(ui, false, false)?;
                native_choose_appearance(ui, true, true)?;
                native_choose_appearance(ui, false, false)?;
            }
            10 => {
                let stored = Index::open(&self.fixture.cache.join("index.db"))?.load_settings()?;
                if stored.english || stored.light_theme {
                    return Ok(false);
                }
                snapshot(ui, &self.output, "settings-dark-zh.png")?;
                ensure!(self.fixture.root.join("late.bin").exists());
                self.checks
                    .push("切回中文深色，文件名和路径保持原样".into());
                native_choose_appearance(ui, true, true)?;
            }
            11 => {
                let settings =
                    Index::open(&self.fixture.cache.join("index.db"))?.load_settings()?;
                if !settings.english || !settings.light_theme {
                    return Ok(false);
                }
                ui.invoke_add_source(0);
            }
            12 => {
                if ui.get_scanning() || ui.get_space_bytes().as_str() != "73" {
                    return Ok(false);
                }
                ensure!(ui.get_page() == 6 && ui.get_space_tiles().row_count() > 0);
                ensure!(
                    ui.get_space_scopes()
                        .row_data(ui.get_space_index().max(0) as usize)
                        .unwrap()
                        .contains("自动分析新增目录")
                );
                ensure!(
                    Index::open(&self.fixture.cache.join("index.db"))?
                        .load_settings()?
                        .sources
                        .iter()
                        .any(|s| minecraft_explorer::platform::path_key(&s.path)
                            == minecraft_explorer::platform::path_key(&self.fixture.extra_root))
                );
                snapshot(ui, &self.output, "automatically-analyzed-new-source.png")?;
                self.checks
                    .push("设置添加目录后自动扫描与该目录空间分析；无需手动点击分析".into());
                fs::write(self.output.join("picker-cancel.marker"), b"fixture")?;
                navigate(ui, 8);
                ui.invoke_add_source(0);
            }
            13 => {
                if ui.get_page() != 6 || !ui.get_scanning() {
                    return Ok(false);
                }
                ui.invoke_cancel_scan();
            }
            14 => {
                if ui.get_scanning() {
                    return Ok(false);
                }
                ensure!(
                    ui.get_space_summary().contains("cancelled")
                        && ui.get_space_tiles().row_count() == 0,
                    "取消后仍执行了自动分析"
                );
                self.checks
                    .push("取消新增目录扫描时跳过自动分析；目录关联仍保存".into());
                return Ok(true);
            }
            _ => return Ok(true),
        }
        self.stage += 1;
        Ok(false)
    }
}

pub fn start(ui: &AppWindow) -> Option<Timer> {
    let output = fs::canonicalize(PathBuf::from(std::env::var_os(
        "MINECRAFT_EXPLORER_UI_STAGE4",
    )?))
    .expect("阶段 4 输出目录");
    assert_eq!(
        fs::read(output.join("stage4-fixture.marker")).unwrap(),
        b"minecraft-explorer-generated-stage4-fixture"
    );
    let fixture: Fixture =
        serde_json::from_slice(&fs::read(output.join("fixture.json")).unwrap()).unwrap();
    assert!(
        fs::canonicalize(&fixture.root)
            .unwrap()
            .starts_with(&output)
    );
    assert!(
        fs::canonicalize(&fixture.cache)
            .unwrap()
            .starts_with(&output)
    );
    let mut run = Run {
        output,
        fixture,
        stage: 0,
        started: Instant::now(),
        checks: Vec::new(),
        verify: std::env::var_os("MINECRAFT_EXPLORER_UI_STAGE4_VERIFY").is_some(),
        linked: false,
        linked_id: slint::SharedString::default(),
    };
    let weak = ui.as_weak();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated,Duration::from_millis(240),move||{
        let Some(ui)=weak.upgrade()else{return;};
        match run.step(&ui){
            Ok(false)=>(),
            result=>{
                let result=match result{Ok(_)=>serde_json::json!({"passed":true,"checks":run.checks,"elapsed_seconds":run.started.elapsed().as_secs_f64()}),Err(error)=>serde_json::json!({"passed":false,"stage":run.stage,"error":format!("{error:#}"),"status":ui.get_status().as_str(),"space_summary":ui.get_space_summary().as_str(),"space_bytes":ui.get_space_bytes().as_str(),"checks":run.checks})};
                fs::write(run.output.join(if run.verify{"restart-result.json"}else{"ui-result.json"}),serde_json::to_vec_pretty(&result).unwrap()).unwrap();
                slint::quit_event_loop().unwrap();
            }
        }
    });
    Some(timer)
}
