use crate::models::AppConfig;
use std::path::Path;

pub fn load_config(path: &Path) -> Result<AppConfig, String> {
    if !path.exists() {
        let config = AppConfig::default();
        save_config(path, &config)?;
        return Ok(config);
    }

    let text = std::fs::read_to_string(path).map_err(|err| format!("读取配置失败：{err}"))?;
    serde_json::from_str(&text).map_err(|err| format!("解析配置失败：{err}"))
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|err| format!("序列化配置失败：{err}"))?;
    std::fs::write(path, text).map_err(|err| format!("保存配置失败：{err}"))
}
