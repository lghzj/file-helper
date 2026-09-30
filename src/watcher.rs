use crate::models::AppConfig;
use notify::event::{CreateKind, DataChange, ModifyKind, RenameMode};
use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct FileChange {
    pub path: PathBuf,
}

pub struct WatchHandle {
    stop_tx: Sender<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl WatchHandle {
    pub fn stop(mut self) {
        let _ = self.stop_tx.send(());
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn start_watcher(config: AppConfig, tx: Sender<FileChange>) -> Result<WatchHandle, String> {
    if config.watch_dirs.is_empty() {
        return Err("请先在设置中添加监听目录".to_owned());
    }
    let valid_dirs: Vec<String> = config.watch_dirs.iter().filter(|dir| Path::new(dir).is_dir()).cloned().collect();
    if valid_dirs.is_empty() {
        return Err("监听目录不存在，请在设置中检查目录路径".to_owned());
    }

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let join = thread::spawn(move || {
        let baseline = collect_baseline(&valid_dirs, config.recursive);
        let (event_tx, event_rx) = mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(event_tx, Config::default()) {
            Ok(watcher) => watcher,
            Err(_) => return,
        };

        let mode = if config.recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        for dir in &valid_dirs {
            let _ = watcher.watch(Path::new(dir), mode);
        }

        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }

            match event_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Ok(event)) => {
                    if !is_relevant_event(&event.kind) {
                        continue;
                    }
                    for path in event.paths.into_iter().filter(|path| is_docx(path)) {
                        if is_unchanged_baseline_file(&baseline, &path) {
                            continue;
                        }
                        let _ = tx.send(FileChange { path });
                    }
                }
                Ok(Err(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    Ok(WatchHandle { stop_tx, join: Some(join) })
}

fn is_relevant_event(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(CreateKind::Any | CreateKind::File) |
            EventKind::Modify(ModifyKind::Data(DataChange::Any | DataChange::Size | DataChange::Content)) |
            EventKind::Modify(ModifyKind::Name(RenameMode::Any | RenameMode::To | RenameMode::Both)) |
            EventKind::Any
    )
}

fn is_docx(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.starts_with("~$") {
        return false;
    }
    path.extension().and_then(|ext| ext.to_str()).map(|ext| ext.eq_ignore_ascii_case("docx")).unwrap_or(false)
}

fn collect_baseline(dirs: &[String], recursive: bool) -> HashMap<PathBuf, std::time::SystemTime> {
    let mut baseline = HashMap::new();
    for dir in dirs {
        collect_docx_files(Path::new(dir), recursive, &mut baseline);
    }
    baseline
}

fn collect_docx_files(path: &Path, recursive: bool, baseline: &mut HashMap<PathBuf, std::time::SystemTime>) {
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && recursive {
            collect_docx_files(&path, recursive, baseline);
            continue;
        }
        if !is_docx(&path) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        baseline.insert(normalize_path(path), modified);
    }
}

fn is_unchanged_baseline_file(baseline: &HashMap<PathBuf, std::time::SystemTime>, path: &Path) -> bool {
    let normalized = normalize_path(path.to_path_buf());
    let Some(start_modified) = baseline.get(&normalized) else {
        return false;
    };
    let Ok(current_modified) = std::fs::metadata(path).and_then(|metadata| metadata.modified()) else {
        return false;
    };
    current_modified <= *start_modified
}

fn normalize_path(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}
