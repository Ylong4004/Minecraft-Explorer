//! Opt-in native-window integration harness; never sends input to other applications.
use crate::AppWindow;
use slint::{
    ComponentHandle, LogicalPosition, Model, Timer, TimerMode,
    platform::{PointerEventButton, WindowEvent},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn click(ui: &AppWindow, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
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
fn snapshot(ui: &AppWindow, output: &std::path::Path, name: &str) {
    let pixels = ui
        .window()
        .take_snapshot()
        .expect("native software-renderer snapshot");
    image::save_buffer(
        output.join(name),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .unwrap();
}
pub fn start(ui: &AppWindow) -> Option<Timer> {
    let output = PathBuf::from(std::env::var_os("MINECRAFT_EXPLORER_UI_SMOKE")?);
    let real = std::env::var_os("MINECRAFT_EXPLORER_UI_REAL").is_some();
    std::fs::create_dir_all(&output).unwrap();
    let timer = Timer::default();
    let weak = ui.as_weak();
    let started = Instant::now();
    let mut stage = 0;
    let mut due = Instant::now();
    timer.start(TimerMode::Repeated,Duration::from_millis(150),move || {
        let ui = weak.upgrade().unwrap();
        assert!(started.elapsed().as_secs()<90,"UI smoke timed out at stage {stage}: {}",ui.get_status());
        if Instant::now()<due { return; }
        match stage {
            0 => {
                if (!real && ui.get_scanning()) || ui.get_entries().row_count()!=if real {206} else {1004} { return; }
                click(&ui,280.,34.);
                for text in if real {["肉","鸽"]} else {["冒","险"]} {
                    ui.window().dispatch_event(WindowEvent::KeyPressed{text:text.into()});
                    ui.window().dispatch_event(WindowEvent::KeyReleased{text:text.into()});
                }
                assert_eq!(ui.get_search().as_str(),if real {"肉鸽"} else {"冒险"},"search field did not receive input");
            }
            1 => {
                if ui.get_entries().row_count()>0 && ui.get_entries().row_data(0).unwrap().icon.size().width==0 {
                    return;
                }
                assert_eq!(ui.get_entries().row_count(),if real {1} else {2},"incorrect search result count");
                snapshot(&ui,&output,"world-search.png");
                click(&ui,270.,215.);
            }
            2 => {
                assert_eq!(ui.get_detail_title().as_str(),if real {"Roguelike Bio"} else {"冒险地图"});
                assert!(ui.get_detail_text().contains(if real {"Roguelike Bio-肉鸽生化[导入包] v1.0"} else {"fabric-custom"}));
                assert_eq!(ui.get_can_open(),real);
                if real { assert_eq!(ui.get_launch_status().as_str(),"OpenOnly"); assert_eq!(ui.get_target_name().as_str(),"Roguelike Bio-肉鸽生化[导入包] v1.0"); }
                snapshot(&ui,&output,"world-details.png");
                click(&ui,58.,483.);
                assert_eq!(ui.get_page(),8,"settings navigation did not respond");
            }
            3 => {
                snapshot(&ui,&output,"settings.png");
                ui.set_about_visible(true);
            }
            4 => {
                snapshot(&ui,&output,"about.png");
                if real {
                    ui.invoke_cancel_scan();
                    std::fs::write(output.join("ui-result.json"),serde_json::json!({"passed":true,"worlds":206,"search_results":1,"real_world_owner_verified":true,"manual_mapping_open_only":true,"settings_navigation":true,"game_launched":false,"elapsed_seconds":started.elapsed().as_secs_f64()}).to_string()).unwrap();
                    slint::quit_event_loop().unwrap();
                    return;
                }
                ui.set_about_visible(false);
                ui.set_search("".into());
                click(&ui,58.,202.);
                ui.invoke_refresh();
            }
            5 => { if !ui.get_scanning() { return; } ui.invoke_cancel_scan(); }
            6 => { if ui.get_scanning() { return; } ui.invoke_refresh(); }
            7 => {
                if ui.get_scanning() { return; }
                assert_eq!(ui.get_entries().row_count(),1004);
                std::fs::write(output.join("ui-result.json"),serde_json::json!({"passed":true,"search_results":2,"worlds":1004,"detail_owner_verified":true,"settings_navigation":true,"cancel_retry":true,"elapsed_seconds":started.elapsed().as_secs_f64()}).to_string()).unwrap();
                slint::quit_event_loop().unwrap();
            }
            _=> unreachable!()
        }
        stage+=1;
        due=Instant::now()+Duration::from_millis(if stage==5 || stage==6 {0} else {850});
    });
    Some(timer)
}
