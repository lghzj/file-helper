use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub base_dir: PathBuf,
    pub config_path: PathBuf,
    pub db_path: PathBuf,
    pub logs_dir: PathBuf,
    pub portable: bool,
}

pub fn resolve_app_paths() -> std::io::Result<AppPaths> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let portable_marker = exe_dir.join("file-helper.portable");
    let portable = portable_marker.exists();

    let base_dir = if portable {
        exe_dir
    } else {
        dirs::data_local_dir()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
            .join("FileHelper")
    };

    let data_dir = base_dir.join("data");
    let logs_dir = base_dir.join("logs");
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&logs_dir)?;

    Ok(AppPaths {
        config_path: base_dir.join("config.json"),
        db_path: data_dir.join("app.db"),
        logs_dir,
        base_dir,
        portable,
    })
}
