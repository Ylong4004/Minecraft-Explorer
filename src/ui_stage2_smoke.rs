//! Opt-in stage 2 native-window integration checks over an isolated fixture.
//! Native events target this Slint window only; no OS input or launcher is sent.
use crate::AppWindow;
use anyhow::{Context, Result, bail, ensure};
use slint::{
    ComponentHandle, LogicalPosition, Model, Timer, TimerMode,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(serde::Deserialize)]
struct Fixture {
    root: PathBuf,
    cache: PathBuf,
    pcl_ce: PathBuf,
}

struct Run {
    output: PathBuf,
    fixture: Fixture,
    verify: bool,
    stage: usize,
    started: Instant,
    stage_started: Instant,
    due: Instant,
    checks: Vec<String>,
}

fn click(ui: &AppWindow, position: LogicalPosition) {
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
}

fn snapshot(ui: &AppWindow, output: &Path, name: &str) -> Result<()> {
    let pixels = ui
        .window()
        .take_snapshot()
        .context("native renderer snapshot")?;
    image::save_buffer(
        output.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(())
}

fn native_search(ui: &AppWindow, value: &str) -> Result<()> {
    // Only the stable top search row needs a logical position. List selection
    // below finds the rendered row, so extra filter rows do not shift its target.
    let width = ui.window().size().width as f32 / ui.window().scale_factor();
    click(ui, LogicalPosition::new(width * 0.4, 34.));
    for ch in value.chars() {
        let text: slint::SharedString = ch.to_string().into();
        ui.window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        ui.window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
    ensure!(
        ui.get_search().as_str() == value,
        "native search field did not receive input"
    );
    Ok(())
}

fn native_select_first(ui: &AppWindow) -> Result<()> {
    let expected = ui.get_entries().row_data(0).context("no first row")?.id;
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(0., 0.),
    });
    let pixels = ui.window().take_snapshot().context("list row snapshot")?;
    let x = pixels.width() / 2;
    let mut run_start = None;
    let mut target = None;
    for y in 80..pixels.height() {
        let pixel = pixels.as_slice()[(y * pixels.width() + x) as usize];
        let row_background = (pixel.r, pixel.g, pixel.b) == (0x17, 0x1f, 0x2a);
        if row_background {
            let first = *run_start.get_or_insert(y);
            if y - first >= 24 {
                target = Some(first + 12);
                break;
            }
        } else {
            run_start = None;
        }
    }
    let y = target.context("visible first-row background not found in rendered image")?;
    let scale = ui.window().scale_factor();
    click(ui, LogicalPosition::new(x as f32 / scale, y as f32 / scale));
    ensure!(
        ui.get_selected_id() == expected,
        "native row selection picked another item"
    );
    Ok(())
}

fn navigate(ui: &AppWindow, page: i32) {
    ui.set_search("".into());
    ui.set_favorite_only(false);
    ui.set_tag_filter("".into());
    ui.set_filter_index(0);
    ui.set_source_index(0);
    ui.set_page(page);
    ui.invoke_navigate(page);
}

fn category_ready(ui: &AppWindow, count: usize, kind: &str) -> bool {
    let rows = ui.get_entries();
    rows.row_count() == count && rows.iter().all(|row| row.kind.as_str() == kind)
}

fn first_icon_ready(ui: &AppWindow) -> bool {
    ui.get_entries()
        .row_data(0)
        .is_some_and(|row| !row.icon_key.is_empty() && row.icon.size().width > 0)
}

fn select_first(ui: &AppWindow) -> Result<()> {
    let row = ui.get_entries().row_data(0).context("expected first row")?;
    ui.set_selected_id(row.id.clone());
    ui.invoke_selected(row.id);
    Ok(())
}

fn detail_ready(ui: &AppWindow, markers: &[&str]) -> bool {
    ui.get_has_icon()
        && markers
            .iter()
            .all(|marker| ui.get_detail_text().contains(marker))
}

impl Run {
    fn database(&self) -> Result<rusqlite::Connection> {
        let connection = rusqlite::Connection::open_with_flags(
            self.fixture.cache.join("index.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        connection.busy_timeout(Duration::from_millis(500))?;
        Ok(connection)
    }

    fn annotation_saved(&self) -> Result<bool> {
        let connection = self.database()?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM annotations a INNER JOIN items i ON i.id=a.item_id
             WHERE i.kind='mod' AND i.name='Stage2 Fabric' AND a.favorite=1
             AND a.tags_json LIKE '%主模组%' AND a.last_opened_ms IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    fn ce_saved(&self) -> Result<bool> {
        let connection = self.database()?;
        let json: String =
            connection.query_row("SELECT json FROM settings WHERE id=1", [], |row| row.get(0))?;
        let value: serde_json::Value = serde_json::from_str(&json)?;
        Ok(value["launchers"]
            .as_array()
            .is_some_and(|launchers| launchers.len() == 1 && launchers[0]["kind"] == "PclCe"))
    }

    fn added(&self) -> PathBuf {
        self.fixture.root.join("resourcepacks").join("Stage2 Added")
    }

    fn capture(&self, ui: &AppWindow, name: &str) -> Result<()> {
        let phase = if self.verify { "verify" } else { "exercise" };
        snapshot(ui, &self.output, &format!("{phase}-{name}.png"))
    }

    fn advance(&mut self, check: &str) {
        self.checks.push(check.to_string());
        self.stage += 1;
        self.stage_started = Instant::now();
        self.due = Instant::now() + Duration::from_millis(500);
    }

    fn step(&mut self, ui: &AppWindow) -> Result<bool> {
        ensure!(
            self.started.elapsed() < Duration::from_secs(180),
            "stage 2 UI total timeout"
        );
        ensure!(
            self.stage_started.elapsed() < Duration::from_secs(30),
            "stage {} timed out: {}",
            self.stage,
            ui.get_status()
        );
        if Instant::now() < self.due {
            return Ok(false);
        }
        if self.verify {
            return self.verify_step(ui);
        }
        match self.stage {
            0 => {
                if ui.get_scanning()
                    || !category_ready(ui, 1, "存档")
                    || !ui.get_counts().contains("2 个资源包")
                {
                    return Ok(false);
                }
                ensure!(ui.get_watch_enabled(), "watcher is disabled in fixture");
                navigate(ui, 3);
                self.advance("initial_fixture_index");
            }
            1 => {
                if !category_ready(ui, 2, "资源包") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                self.capture(ui, "resource-list")?;
                native_select_first(ui)?;
                self.advance("resource_category_and_visible_row_icon");
            }
            2 => {
                if !detail_ready(ui, &["Stage2 textures", "pack_format = 15"]) {
                    return Ok(false);
                }
                self.capture(ui, "resource-details")?;
                navigate(ui, 4);
                self.advance("resource_metadata_and_png_preview");
            }
            3 => {
                if !category_ready(ui, 1, "模组") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                native_search(ui, "stage2_fabric")?;
                self.advance("native_search_input");
            }
            4 => {
                if !category_ready(ui, 1, "模组")
                    || !first_icon_ready(ui)
                    || !ui
                        .get_entries()
                        .row_data(0)
                        .unwrap()
                        .title
                        .contains("Stage2 Fabric")
                {
                    return Ok(false);
                }
                self.capture(ui, "mod-search")?;
                native_select_first(ui)?;
                self.advance("native_filtered_list_selection");
            }
            5 => {
                if !detail_ready(
                    ui,
                    &["stage2_fabric", "2.0", "Stage2 author", "Fabric", "1.21.x"],
                ) {
                    return Ok(false);
                }
                ensure!(
                    ui.get_detail_title().as_str() == "Stage2 Fabric",
                    "mod display name did not come from metadata"
                );
                self.capture(ui, "mod-details")?;
                ui.set_detail_favorite(true);
                ui.set_detail_tags("Stage2, 主模组".into());
                ui.invoke_annotate_selected();
                self.advance("mod_declared_metadata_and_annotation_save");
            }
            6 => {
                if !self.annotation_saved()? {
                    return Ok(false);
                }
                navigate(ui, 10);
                self.advance("annotation_persisted_in_application_database");
            }
            7 => {
                if !category_ready(ui, 1, "模组")
                    || !ui.get_entries().row_data(0).unwrap().title.starts_with('★')
                {
                    return Ok(false);
                }
                select_first(ui)?;
                self.advance("favorites_filter");
            }
            8 => {
                if !ui.get_detail_favorite() || !ui.get_detail_tags().contains("主模组") {
                    return Ok(false);
                }
                self.capture(ui, "favorites")?;
                ui.set_tag_filter("missing-tag".into());
                self.advance("annotation_readback_in_details");
            }
            9 => {
                if ui.get_entries().row_count() != 0 {
                    return Ok(false);
                }
                self.capture(ui, "tag-empty-result")?;
                ui.set_tag_filter("主模组".into());
                self.advance("nonmatching_exact_tag_filter");
            }
            10 => {
                if !category_ready(ui, 1, "模组") {
                    return Ok(false);
                }
                self.capture(ui, "tag-result")?;
                navigate(ui, 11);
                self.advance("matching_exact_tag_filter");
            }
            11 => {
                if ui.get_entries().row_count() < 2
                    || !ui
                        .get_entries()
                        .iter()
                        .any(|row| row.title.contains("Stage2 Fabric"))
                {
                    return Ok(false);
                }
                ensure!(
                    ui.get_entries()
                        .row_data(0)
                        .unwrap()
                        .title
                        .contains("Stage2 Fabric"),
                    "recent order did not put the most recently viewed mod first"
                );
                self.capture(ui, "recent")?;
                navigate(ui, 5);
                self.advance("recent_viewed_entries_and_order");
            }
            12 => {
                if !category_ready(ui, 1, "光影包") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                select_first(ui)?;
                self.advance("shader_category_and_visible_row_icon");
            }
            13 => {
                if !detail_ready(
                    ui,
                    &[
                        "Stage2 shaders",
                        "min_format = [69,0]",
                        "max_format = [69,1]",
                    ],
                ) {
                    return Ok(false);
                }
                self.capture(ui, "shader-details")?;
                navigate(ui, 9);
                self.advance("shader_explicit_metadata_and_png_preview");
            }
            14 => {
                if !category_ready(ui, 1, "数据包") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                select_first(ui)?;
                self.advance("world_datapack_category_and_visible_row_icon");
            }
            15 => {
                if !detail_ready(ui, &["Stage2 data", "pack_format = 88"]) {
                    return Ok(false);
                }
                self.capture(ui, "datapack-details")?;
                navigate(ui, 12);
                ui.invoke_find_duplicates();
                self.advance("datapack_declared_metadata_and_png_preview");
            }
            16 => {
                if ui.get_scanning()
                    || !ui
                        .get_duplicate_summary()
                        .contains("1 个重复候选组 / 2 个条目")
                    || !category_ready(ui, 2, "资源包")
                {
                    return Ok(false);
                }
                ensure!(
                    ui.get_duplicate_summary().contains("未删除文件"),
                    "duplicate result omits read-only status"
                );
                self.capture(ui, "duplicate-candidates")?;
                navigate(ui, 3);
                fs::create_dir(self.added())?;
                fs::write(
                    self.added().join("pack.mcmeta"),
                    br#"{"pack":{"description":"Automatic creation","pack_format":15}}"#,
                )?;
                fs::copy(
                    self.fixture
                        .root
                        .join("saves")
                        .join("Stage2World")
                        .join("icon.png"),
                    self.added().join("pack.png"),
                )?;
                self.advance("read_only_duplicate_candidates_and_fixture_creation");
            }
            17 => {
                if !category_ready(ui, 3, "资源包") {
                    return Ok(false);
                }
                let Some(row) = ui
                    .get_entries()
                    .iter()
                    .find(|row| row.title.contains("Stage2 Added"))
                else {
                    return Ok(false);
                };
                ui.set_selected_id(row.id.clone());
                ui.invoke_selected(row.id);
                self.advance("native_directory_notification_adds_item_without_refresh");
            }
            18 => {
                if !detail_ready(ui, &["Automatic creation"]) {
                    return Ok(false);
                }
                self.capture(ui, "watch-created")?;
                fs::write(self.added().join("pack.mcmeta"), br#"{"pack":{"description":"Automatic modification","min_format":[69,0],"max_format":[69,1]}}"#)?;
                self.advance("native_directory_notification_reads_created_metadata");
            }
            19 => {
                if !detail_ready(ui, &["Automatic modification", "max_format = [69,1]"]) {
                    return Ok(false);
                }
                self.capture(ui, "watch-modified")?;
                fs::remove_file(self.added().join("pack.mcmeta"))?;
                fs::remove_file(self.added().join("pack.png"))?;
                fs::remove_dir(self.added())?;
                self.advance("native_directory_notification_updates_metadata_without_refresh");
            }
            20 => {
                if !category_ready(ui, 2, "资源包")
                    || ui
                        .get_entries()
                        .iter()
                        .any(|row| row.title.contains("Stage2 Added"))
                {
                    return Ok(false);
                }
                self.capture(ui, "watch-removed")?;
                navigate(ui, 8);
                ui.invoke_edit_launcher("".into());
                self.advance("native_directory_notification_removes_item_without_refresh");
            }
            21 => {
                ensure!(ui.get_editor_visible(), "launcher editor did not open");
                ui.set_edit_kind(2);
                ui.set_edit_executable(self.fixture.pcl_ce.to_string_lossy().to_string().into());
                ui.set_edit_workdir(
                    self.fixture
                        .pcl_ce
                        .parent()
                        .context("fixture launcher parent")?
                        .to_string_lossy()
                        .to_string()
                        .into(),
                );
                ui.set_edit_java("".into());
                ui.set_edit_root_index(0);
                self.capture(ui, "pcl-ce-editor")?;
                ui.invoke_save_launcher();
                self.advance("pcl_ce_editor_kind_index_2_save");
            }
            22 => {
                if ui.get_editor_visible()
                    || ui.get_launchers().row_count() != 1
                    || !self.ce_saved()?
                {
                    return Ok(false);
                }
                ensure!(
                    ui.get_launchers()
                        .row_data(0)
                        .unwrap()
                        .label
                        .contains("PCL CE"),
                    "saved launcher label is not PCL CE"
                );
                self.capture(ui, "pcl-ce-settings")?;
                ui.invoke_refresh();
                self.advance("pcl_ce_configuration_persisted_without_launching");
            }
            23 => {
                if ui.get_scanning() {
                    return Ok(false);
                }
                ensure!(
                    self.annotation_saved()?,
                    "full refresh discarded favorite, tags or recent record"
                );
                navigate(ui, 10);
                self.advance("annotation_survives_full_refresh");
            }
            24 => {
                if !category_ready(ui, 1, "模组") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                select_first(ui)?;
                self.advance("favorite_still_visible_after_refresh");
            }
            25 => {
                if !ui.get_detail_favorite() || !ui.get_detail_tags().contains("主模组") {
                    return Ok(false);
                }
                self.capture(ui, "final-favorite")?;
                self.checks.push("annotation_readback_after_refresh".into());
                return Ok(true);
            }
            _ => bail!("unexpected stage {}", self.stage),
        }
        Ok(false)
    }

    fn verify_step(&mut self, ui: &AppWindow) -> Result<bool> {
        match self.stage {
            0 => {
                if ui.get_scanning() || !category_ready(ui, 1, "存档") {
                    return Ok(false);
                }
                ensure!(
                    self.annotation_saved()?,
                    "annotations missing after app restart"
                );
                ensure!(
                    self.ce_saved()?,
                    "PCL CE installation missing after app restart"
                );
                navigate(ui, 10);
                self.advance("restart_database_state");
            }
            1 => {
                if !category_ready(ui, 1, "模组") || !first_icon_ready(ui) {
                    return Ok(false);
                }
                native_select_first(ui)?;
                self.advance("restart_favorite_filter_and_native_selection");
            }
            2 => {
                if !detail_ready(ui, &["stage2_fabric", "2.0"])
                    || !ui.get_detail_favorite()
                    || !ui.get_detail_tags().contains("主模组")
                {
                    return Ok(false);
                }
                self.capture(ui, "persisted-favorite")?;
                navigate(ui, 11);
                self.advance("restart_annotation_and_icon_readback");
            }
            3 => {
                if ui.get_entries().row_count() < 4
                    || !ui
                        .get_entries()
                        .iter()
                        .any(|row| row.title.contains("Stage2 Fabric"))
                {
                    return Ok(false);
                }
                self.capture(ui, "persisted-recent")?;
                navigate(ui, 8);
                let launcher = ui
                    .get_launchers()
                    .row_data(0)
                    .context("persisted launcher row")?;
                ui.invoke_edit_launcher(launcher.id);
                self.advance("restart_recent_entries");
            }
            4 => {
                if !ui.get_editor_visible() {
                    return Ok(false);
                }
                ensure!(
                    ui.get_edit_kind() == 2,
                    "PCL CE reopened as another launcher kind"
                );
                ensure!(
                    ui.get_edit_executable().contains("PCL-CE-fixture.exe"),
                    "PCL CE executable was not restored"
                );
                self.capture(ui, "persisted-pcl-ce-editor")?;
                ui.set_editor_visible(false);
                self.checks
                    .push("restart_pcl_ce_editor_kind_index_2".into());
                return Ok(true);
            }
            _ => bail!("unexpected verify stage {}", self.stage),
        }
        Ok(false)
    }

    fn finish(&self, ui: &AppWindow, error: Option<String>) {
        let phase = if self.verify { "verify" } else { "exercise" };
        if error.is_some() {
            let _ = self.capture(ui, &format!("failure-stage-{}", self.stage));
        }
        let result = serde_json::json!({
            "passed": error.is_none(), "phase": phase, "stage": self.stage,
            "checks": self.checks, "error": error, "status": ui.get_status().as_str(),
            "elapsed_seconds": self.started.elapsed().as_secs_f64(),
            "native_input_target": "this_app_window_only", "launcher_opened": false,
            "game_launched": false,
        });
        let _ = fs::write(
            self.output.join(format!("stage2-ui-{phase}.json")),
            result.to_string(),
        );
        let _ = slint::quit_event_loop();
    }
}

pub fn start(ui: &AppWindow) -> Option<Timer> {
    let output = PathBuf::from(std::env::var_os("MINECRAFT_EXPLORER_UI_STAGE2")?);
    let verify = std::env::var_os("MINECRAFT_EXPLORER_UI_STAGE2_VERIFY").is_some();
    let setup = (|| -> Result<Run> {
        let output = fs::canonicalize(output)?;
        let fixture: Fixture = serde_json::from_slice(&fs::read(output.join("fixture.json"))?)?;
        ensure!(
            fs::canonicalize(&fixture.root)?.starts_with(&output)
                && fs::canonicalize(&fixture.cache)?.starts_with(&output)
                && fs::canonicalize(&fixture.pcl_ce)?.starts_with(&output),
            "UI fixture paths must all remain inside the dedicated output directory"
        );
        ensure!(
            fs::read(output.join("stage2-fixture.marker"))?
                == b"minecraft-explorer-generated-stage2-fixture",
            "generated fixture marker missing"
        );
        let now = Instant::now();
        Ok(Run {
            output,
            fixture,
            verify,
            stage: 0,
            started: now,
            stage_started: now,
            due: now + Duration::from_millis(350),
            checks: Vec::new(),
        })
    })();
    let mut run = match setup {
        Ok(run) => run,
        Err(error) => {
            eprintln!("stage 2 UI fixture setup failed: {error:#}");
            let _ = slint::quit_event_loop();
            return None;
        }
    };
    let timer = Timer::default();
    let weak = ui.as_weak();
    let mut finished = false;
    timer.start(TimerMode::Repeated, Duration::from_millis(150), move || {
        if finished {
            return;
        }
        let Some(ui) = weak.upgrade() else {
            return;
        };
        match run.step(&ui) {
            Ok(false) => {}
            Ok(true) => {
                finished = true;
                run.finish(&ui, None);
            }
            Err(error) => {
                finished = true;
                run.finish(&ui, Some(format!("{error:#}")));
            }
        }
    });
    Some(timer)
}
