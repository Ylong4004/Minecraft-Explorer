fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let version = std::env::var("CARGO_PKG_VERSION").expect("缺少包版本");
        let numbers = ["MAJOR", "MINOR", "PATCH"].map(|part| {
            std::env::var(format!("CARGO_PKG_VERSION_{part}")).expect("缺少包版本数字")
        });
        let numbers = format!("{},{},{},0", numbers[0], numbers[1], numbers[2]);
        let macros = [
            format!("APP_VERSION_NUM={numbers}"),
            format!("APP_VERSION_TEXT=\"{version}\""),
        ];
        embed_resource::compile_for("assets/app.rc", ["minecraft-explorer"], &macros)
            .manifest_optional()
            .expect("Windows 图标资源编译失败");
    }
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new().with_style("fluent-dark".into()),
    )
    .expect("Slint 界面编译失败");
}
