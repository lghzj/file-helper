use chrono::Local;
use std::path::Path;

pub fn append_log(logs_dir: &Path, level: &str, message: &str) -> Result<(), String> {
    std::fs::create_dir_all(logs_dir).map_err(|err| format!("创建日志目录失败：{err}"))?;
    let today = Local::now().format("%Y-%m-%d").to_string();
    let path = logs_dir.join(format!("app-{today}.log"));
    let line = format!("{} [{level}] {message}\n", Local::now().format("%Y-%m-%d %H:%M:%S"));
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()))
        .map_err(|err| format!("写入日志失败：{err}"))
}

pub fn prune_logs(logs_dir: &Path, retention_days: u32) -> Result<usize, String> {
    if !logs_dir.exists() {
        return Ok(0);
    }
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(retention_days.max(1) as u64 * 24 * 60 * 60))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let mut removed = 0;
    for entry in std::fs::read_dir(logs_dir).map_err(|err| format!("读取日志目录失败：{err}"))? {
        let entry = entry.map_err(|err| format!("读取日志文件失败：{err}"))?;
        let path = entry.path();
        let is_log = path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.eq_ignore_ascii_case("log")).unwrap_or(false);
        if !is_log {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified < cutoff {
            if std::fs::remove_file(path).is_ok() {
                removed += 1;
            }
        }
    }
    Ok(removed)
}
