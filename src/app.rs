use crate::config::{load_config, save_config};
use crate::api::submit_document;
use crate::logging::{append_log, prune_logs};
use crate::models::{AppConfig, ParseRecord, ParseStatus};
use crate::paths::resolve_app_paths;
use crate::plugins::parse_with_builtin;
use crate::storage::Storage;
use crate::watcher::{start_watcher, FileChange, WatchHandle};
use chrono::DateTime;
use chrono::Local;
use eframe::egui::{
    self, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Vec2,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const ACCENT: Color32 = Color32::from_rgb(31, 105, 196);
const TOOL_BG: Color32 = Color32::from_rgb(239, 243, 248);
const LINE: Color32 = Color32::from_rgb(205, 211, 218);
const GRID_LINE: Color32 = Color32::from_rgb(231, 231, 231);
const TABLE_BORDER: Color32 = Color32::from_rgb(145, 151, 158);
const HEADER_BG: Color32 = Color32::from_rgb(252, 252, 252);
const ROW_ALT_BG: Color32 = Color32::from_rgb(252, 252, 252);
const ROW_SELECTED_BG: Color32 = Color32::from_rgb(205, 232, 255);

pub struct FileHelperApp {
    config: AppConfig,
    records: Vec<ParseRecord>,
    selected_record_id: Option<u64>,
    listening: bool,
    show_settings: bool,
    show_detail: bool,
    detail_json_mode: JsonMode,
    draft_watch_dir: String,
    status_message: String,
    config_path: Option<PathBuf>,
    logs_dir_path: Option<PathBuf>,
    base_dir_text: String,
    logs_dir_text: String,
    storage: Option<Storage>,
    watcher: Option<WatchHandle>,
    change_rx: Option<Receiver<FileChange>>,
    pending_changes: HashMap<PathBuf, Instant>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonMode {
    Raw,
    Final,
}

impl FileHelperApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_chinese_font(&cc.egui_ctx);
        cc.egui_ctx.set_pixels_per_point(1.0);

        let mut config = AppConfig::default();
        let mut records = Vec::new();
        let mut config_path = None;
        let mut logs_dir_path = None;
        let mut storage = None;
        let mut base_dir_text = "-".to_owned();
        let mut logs_dir_text = "-".to_owned();
        let mut status_message = "监听中，等待 Word 文件变化".to_owned();

        match resolve_app_paths() {
            Ok(paths) => {
                base_dir_text = paths.base_dir.display().to_string();
                logs_dir_text = paths.logs_dir.display().to_string();
                config_path = Some(paths.config_path.clone());
                logs_dir_path = Some(paths.logs_dir.clone());
                match load_config(&paths.config_path) {
                    Ok(loaded) => config = loaded,
                    Err(err) => status_message = err,
                }

                match Storage::open(&paths.db_path) {
                    Ok(db) => {
                        if let Err(err) = db.prune_records(config.retention_days) {
                            status_message = err;
                        }
                        match db.list_records() {
                            Ok(loaded_records) => records = loaded_records,
                            Err(err) => status_message = err,
                        }
                        storage = Some(db);
                    }
                    Err(err) => status_message = err,
                }

                if paths.portable {
                    status_message = format!("便携模式，数据目录：{}", base_dir_text);
                }
                if let Some(logs_dir) = &logs_dir_path {
                    let _ = prune_logs(logs_dir, config.retention_days);
                }
            }
            Err(err) => {
                status_message = format!("初始化应用目录失败：{err}");
            }
        }

        Self {
            config,
            selected_record_id: records.first().map(|record| record.id),
            records,
            listening: false,
            show_settings: false,
            show_detail: false,
            detail_json_mode: JsonMode::Final,
            draft_watch_dir: String::new(),
            status_message,
            config_path,
            base_dir_text,
            logs_dir_text,
            logs_dir_path,
            storage,
            watcher: None,
            change_rx: None,
            pending_changes: HashMap::new(),
        }
    }

    fn selected_record(&self) -> Option<&ParseRecord> {
        let id = self.selected_record_id?;
        self.records.iter().find(|record| record.id == id)
    }

    fn selected_record_mut(&mut self) -> Option<&mut ParseRecord> {
        let id = self.selected_record_id?;
        self.records.iter_mut().find(|record| record.id == id)
    }

    fn success_count(&self) -> usize {
        self.records.iter().filter(|record| record.status == ParseStatus::Success).count()
    }

    fn failed_count(&self) -> usize {
        self.records.iter().filter(|record| record.status == ParseStatus::Failed).count()
    }

    fn reparse_selected(&mut self) {
        let parser_id = self.config.parser_id.clone();
        let transformer_id = self.config.transformer_id.clone();
        let mut changed = None;
        let mut changed_file_name = None;
        if let Some(record) = self.selected_record_mut() {
            let now = Local::now().format("%H:%M:%S").to_string();
            record.status = ParseStatus::Success;
            record.parsed_at = now;
            record.parser_id = parser_id;
            record.transformer_id = transformer_id;
            record.error_message.clear();
            changed_file_name = Some(record.file_name.clone());
            changed = Some(record.clone());
        }
        if let Some(file_name) = changed_file_name {
            self.status_message = format!("已重新解析 {file_name}");
        }
        if let (Some(storage), Some(record)) = (&self.storage, changed) {
            if let Err(err) = storage.update_record(&record) {
                self.status_message = err;
            }
        }
    }

    fn delete_selected(&mut self) {
        if let Some(id) = self.selected_record_id {
            if let Some(storage) = &self.storage {
                if let Err(err) = storage.delete_record(id) {
                    self.status_message = err;
                    return;
                }
            }
            self.records.retain(|record| record.id != id);
            self.selected_record_id = self.records.first().map(|record| record.id);
            self.status_message = "已删除选中记录".to_owned();
        }
    }

    fn save_current_config(&mut self) {
        let Some(path) = &self.config_path else {
            self.status_message = "配置路径未初始化，无法保存".to_owned();
            return;
        };
        match save_config(path, &self.config) {
            Ok(()) => {
                self.status_message = format!("配置已保存：{}", path.display());
                self.log_info(&self.status_message.clone());
            }
            Err(err) => self.status_message = err,
        }
    }

    fn log_info(&self, message: &str) {
        if let Some(logs_dir) = &self.logs_dir_path {
            let _ = append_log(logs_dir, "INFO", message);
        }
    }

    fn log_error(&self, message: &str) {
        if let Some(logs_dir) = &self.logs_dir_path {
            let _ = append_log(logs_dir, "ERROR", message);
        }
    }

    fn start_listening(&mut self) {
        let (tx, rx) = mpsc::channel();
        match start_watcher(self.config.clone(), tx) {
            Ok(handle) => {
                self.watcher = Some(handle);
                self.change_rx = Some(rx);
                self.listening = true;
                self.status_message = "监听已启动".to_owned();
                self.log_info("监听已启动");
            }
            Err(err) => {
                self.status_message = err;
                self.listening = false;
                self.log_error(&self.status_message);
            }
        }
    }

    fn stop_listening(&mut self) {
        if let Some(handle) = self.watcher.take() {
            handle.stop();
        }
        self.change_rx = None;
        self.pending_changes.clear();
        self.listening = false;
        self.status_message = "监听已暂停".to_owned();
        self.log_info("监听已暂停");
    }

    fn process_pending_changes(&mut self) {
        if let Some(rx) = &self.change_rx {
            while let Ok(change) = rx.try_recv() {
                self.pending_changes.insert(change.path, Instant::now());
            }
        }
        let settle = Duration::from_secs(self.config.settle_seconds.max(1) as u64);
        let now = Instant::now();
        let ready: Vec<PathBuf> = self
            .pending_changes
            .iter()
            .filter_map(|(path, last_seen)| if now.duration_since(*last_seen) >= settle { Some(path.clone()) } else { None })
            .collect();
        for path in ready {
            self.pending_changes.remove(&path);
            self.handle_file_change(FileChange { path });
        }
    }

    fn handle_file_change(&mut self, change: FileChange) {
        let path = change.path;
        let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_owned();
        let detected_at = Local::now().format("%H:%M:%S").to_string();

        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => {
                self.status_message = format!("读取文件属性失败：{err}");
                self.log_error(&self.status_message);
                return;
            }
        };
        let modified_at = metadata.modified().map(format_system_time).unwrap_or_else(|_| "-".to_owned());

        let mut record = ParseRecord {
            id: 0,
            file_name: file_name.clone(),
            file_path: path.display().to_string(),
            file_size: format_file_size(metadata.len()),
            modified_at,
            detected_at,
            parsed_at: "--".to_owned(),
            parser_id: self.config.parser_id.clone(),
            transformer_id: self.config.transformer_id.clone(),
            status: ParseStatus::Parsing,
            error_message: String::new(),
            raw_json: "{}".to_owned(),
            final_json: "{}".to_owned(),
        };

        match parse_with_builtin(&self.config.parser_id, &self.config.transformer_id, &path) {
            Ok(parsed) => {
                record.status = ParseStatus::Success;
                record.parsed_at = Local::now().format("%H:%M:%S").to_string();
                record.raw_json = serde_json::to_string_pretty(&parsed.raw_json).unwrap_or_else(|_| "{}".to_owned());
                record.final_json = serde_json::to_string_pretty(&parsed.final_json).unwrap_or_else(|_| "{}".to_owned());
                self.status_message = format!("解析成功：{file_name}");
                self.log_info(&format!("解析成功：{}", record.file_path));
                if !self.config.api_base_url.trim().is_empty() {
                    match submit_document(&self.config.api_base_url, &self.config.creator_account, &self.config.basic_auth_username, &self.config.basic_auth_password, &path, &record.final_json) {
                        Ok(()) => {
                            self.status_message = format!("解析并提交成功：{file_name}");
                            self.log_info(&format!("接口提交成功：{}", record.file_path));
                        }
                        Err(err) => {
                            self.status_message = format!("解析成功，接口提交失败：{err}");
                            self.log_error(&format!("接口提交失败：{}，{}", record.file_path, err));
                        }
                    }
                }
            }
            Err(err) => {
                record.status = ParseStatus::Failed;
                record.parsed_at = Local::now().format("%H:%M:%S").to_string();
                record.error_message = err;
                self.status_message = format!("解析失败：{file_name}");
                self.log_error(&format!("解析失败：{}，{}", record.file_path, record.error_message));
            }
        }

        if let Some(storage) = &self.storage {
            match storage.insert_record(&record) {
                Ok(id) => record.id = id,
                Err(err) => {
                    self.status_message = err;
                    self.log_error(&self.status_message);
                }
            }
        } else {
            record.id = self.records.iter().map(|record| record.id).max().unwrap_or(0) + 1;
        }
        self.selected_record_id = Some(record.id);
        self.records.insert(0, record);
    }
}

impl Drop for FileHelperApp {
    fn drop(&mut self) {
        if let Some(handle) = self.watcher.take() {
            handle.stop();
        }
    }
}

impl eframe::App for FileHelperApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_pending_changes();

        egui::TopBottomPanel::top("main_toolbar")
            .frame(
                egui::Frame::default()
                    .fill(TOOL_BG)
                    .stroke(egui::Stroke::new(1.0_f32, LINE))
                    .inner_margin(egui::Margin::symmetric(5.0, 4.0)),
            )
            .show(ctx, |ui| self.main_toolbar(ui));
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::default().fill(Color32::from_rgb(246, 247, 249)).stroke(egui::Stroke::new(1.0_f32, LINE)))
            .show(ctx, |ui| self.status_bar(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(4.0);
            self.record_table(ui);
        });

        self.settings_window(ctx);
        self.detail_window(ctx);
    }
}

impl FileHelperApp {
    fn main_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if self.listening {
                if tool_button(ui, "暂停监听").clicked() {
                    self.stop_listening();
                }
            } else if tool_button(ui, "开始监听").clicked() {
                self.start_listening();
            }

            ui.separator();
            if tool_button(ui, "重新解析").clicked() {
                self.reparse_selected();
            }
            if tool_button(ui, "查看详情").clicked() && self.selected_record_id.is_some() {
                self.show_detail = true;
            }
            if tool_button(ui, "删除记录").clicked() {
                self.delete_selected();
            }
            if tool_button(ui, "设置").clicked() {
                self.show_settings = true;
            }
            if tool_button(ui, "关于").clicked() {
                self.status_message = "File Helper v0.1.0，绿色版 Word 解析小工具原型".to_owned();
            }
        });
    }

    fn record_table(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("文件解析记录").strong().size(16.0));
            ui.add_space(8.0);
            ui.label(RichText::new("仅展示监听到变化后的 Word 文件").color(Color32::DARK_GRAY).size(13.0));
        });
        ui.add_space(4.0);

        self.draw_record_grid(ui);
    }

    fn draw_record_grid(&mut self, ui: &mut egui::Ui) {
        let table_width = ui.available_width();
        let header_h = 25.0;
        let row_h = 26.0;
        let min_h = header_h + row_h * self.records.len() as f32;
        let table_h = ui.available_height().max(min_h + 1.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(table_width, table_h), Sense::hover());
        let painter = ui.painter_at(rect);

        let mut widths = [52.0, 230.0, 88.0, 82.0, 130.0, 130.0, 0.0];
        let fixed: f32 = widths[..6].iter().sum();
        widths[6] = (table_width - fixed).max(280.0);

        painter.rect_filled(rect, 0.0, Color32::WHITE);
        let header_rect = Rect::from_min_size(rect.min, Vec2::new(table_width, header_h));
        painter.rect_filled(header_rect, 0.0, HEADER_BG);

        let headers = ["序号", "文件名", "状态", "大小", "修改时间", "解析时间", "错误信息"];
        let font = FontId::proportional(13.0);
        let row_font = FontId::proportional(13.0);
        for (index, title) in headers.iter().enumerate() {
            let cell = table_cell(rect, &widths, index, rect.top(), header_h);
            draw_text_clipped(&painter, cell, title, font.clone(), Color32::from_rgb(35, 40, 48), true);
        }

        for (row_index, record) in self.records.clone().iter().enumerate() {
            let y = rect.top() + header_h + row_h * row_index as f32;
            let row_rect = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(table_width, row_h));
            let selected = self.selected_record_id == Some(record.id);
            let bg = if selected {
                ROW_SELECTED_BG
            } else if row_index % 2 == 0 {
                Color32::WHITE
            } else {
                ROW_ALT_BG
            };
            painter.rect_filled(row_rect, 0.0, bg);

            let response = ui.interact(row_rect, ui.id().with(("record_row", record.id)), Sense::click());
            if response.clicked() {
                self.selected_record_id = Some(record.id);
            }
            if response.double_clicked() {
                self.selected_record_id = Some(record.id);
                self.show_detail = true;
            }

            let sequence = (row_index + 1).to_string();
            let values = [
                sequence.as_str(),
                record.file_name.as_str(),
                record.status.label(),
                record.file_size.as_str(),
                record.modified_at.as_str(),
                record.parsed_at.as_str(),
                record.error_message.as_str(),
            ];

            for (index, value) in values.iter().enumerate() {
                let color = if index == 2 { status_color(record.status) } else { Color32::from_rgb(38, 38, 38) };
                let strong = index == 2;
                let cell = table_cell(rect, &widths, index, y, row_h);
                draw_text_clipped(&painter, cell, value, row_font.clone(), color, strong);
            }
        }

        draw_table_grid(&painter, rect, &widths, header_h, row_h, table_h);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(if self.listening { "监听中" } else { "已暂停" });
            ui.separator();
            ui.label(format!("目录：{}", self.config.watch_dirs.len()));
            ui.separator();
            ui.label(format!("今日：{}", self.records.len()));
            ui.separator();
            ui.label(RichText::new(format!("成功：{}", self.success_count())).color(Color32::from_rgb(0, 120, 64)));
            ui.separator();
            ui.label(RichText::new(format!("失败：{}", self.failed_count())).color(Color32::from_rgb(190, 45, 45)));
            ui.separator();
            ui.label(format!("稳定等待：{} 秒", self.config.settle_seconds));
            ui.separator();
            ui.label(&self.status_message);
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        egui::Window::new("设置")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(680.0)
            .show(ctx, |ui| {
                ui.label(RichText::new(format!("数据目录：{}", self.base_dir_text)).size(12.0).color(Color32::DARK_GRAY));
                ui.label(RichText::new(format!("日志目录：{}", self.logs_dir_text)).size(12.0).color(Color32::DARK_GRAY));
                ui.add_space(8.0);
                draw_section_title(ui, "接口设置");
                egui::Grid::new("api_settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("接口基础路径");
                        ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.config.api_base_url).hint_text("例如 https://example.com/api/upload"));
                        ui.end_row();
                        ui.label("创建人账号");
                        ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.config.creator_account));
                        ui.end_row();
                        ui.label("Basic Auth 用户名");
                        ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.config.basic_auth_username));
                        ui.end_row();
                        ui.label("Basic Auth 密码");
                        ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.config.basic_auth_password).password(true));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                draw_section_title(ui, "常规设置");
                egui::Grid::new("general_settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("开机自启");
                        ui.checkbox(&mut self.config.start_on_boot, "启用");
                        ui.end_row();
                        ui.label("关闭行为");
                        ui.checkbox(&mut self.config.close_to_tray, "关闭窗口时最小化到托盘");
                        ui.end_row();
                        ui.label("记录保留天数");
                        ui.add(egui::DragValue::new(&mut self.config.retention_days).range(1..=30));
                        ui.end_row();
                    });

                ui.add_space(8.0);
                draw_section_title(ui, "监听设置");
                egui::Grid::new("watch_settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("递归监听");
                        ui.checkbox(&mut self.config.recursive, "监听子目录");
                        ui.end_row();
                        ui.label("稳定等待秒数");
                        ui.add(egui::DragValue::new(&mut self.config.settle_seconds).range(1..=120));
                        ui.end_row();
                    });

                ui.add_space(6.0);
                draw_directory_table(ui, &self.config.watch_dirs);
                ui.horizontal(|ui| {
                    ui.label("新增目录");
                    ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.draft_watch_dir));
                    if ui.button("添加").clicked() && !self.draft_watch_dir.trim().is_empty() {
                        self.config.watch_dirs.push(self.draft_watch_dir.trim().to_owned());
                        self.draft_watch_dir.clear();
                    }
                });

                ui.add_space(8.0);
                draw_section_title(ui, "插件与排错");
                egui::Grid::new("plugin_settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Parser");
                        ui.text_edit_singleline(&mut self.config.parser_id);
                        ui.end_row();
                        ui.label("Transformer");
                        ui.text_edit_singleline(&mut self.config.transformer_id);
                        ui.end_row();
                        ui.label("保存 raw_json");
                        ui.checkbox(&mut self.config.save_raw_json, "启用");
                        ui.end_row();
                        ui.label("保存 final_json");
                        ui.checkbox(&mut self.config.save_final_json, "启用");
                        ui.end_row();
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("保存设置").clicked() {
                        self.save_current_config();
                    }
                    ui.label(RichText::new("设置保存后，后续真实监听模块会按新配置重载。").size(12.0).color(Color32::DARK_GRAY));
                });
            });
        self.show_settings = open;
    }

    fn detail_window(&mut self, ctx: &egui::Context) {
        let Some(record) = self.selected_record().cloned() else {
            return;
        };

        let mut open = self.show_detail;
        egui::Window::new(format!("文件详情 - {}", record.file_name))
            .open(&mut open)
            .default_width(760.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                draw_detail_table(ui, &record);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.detail_json_mode, JsonMode::Raw, "raw_json");
                    ui.selectable_value(&mut self.detail_json_mode, JsonMode::Final, "final_json");
                });
                ui.separator();
                let mut json = match self.detail_json_mode {
                    JsonMode::Raw => record.raw_json,
                    JsonMode::Final => record.final_json,
                };
                ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut json)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(24),
                    );
                });
            });
        self.show_detail = open;
    }
}

fn tool_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let text_width = label.chars().count() as f32 * 14.0;
    let size = Vec2::new((text_width + 24.0).max(58.0), 28.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let fill = if response.is_pointer_button_down_on() {
        Color32::from_rgb(215, 228, 244)
    } else if response.hovered() {
        Color32::from_rgb(229, 238, 250)
    } else {
        TOOL_BG
    };
    let stroke = if response.hovered() || response.is_pointer_button_down_on() {
        Stroke::new(1.0_f32, Color32::from_rgb(143, 174, 211))
    } else {
        Stroke::new(1.0_f32, Color32::TRANSPARENT)
    };

    let button_rect = rect.shrink2(Vec2::new(1.0, 1.0));
    ui.painter().rect_filled(button_rect, 2.0, fill);
    ui.painter().rect_stroke(button_rect, 2.0, stroke);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(13.0),
        Color32::from_rgb(45, 48, 54),
    );

    response
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let local: DateTime<Local> = time.into();
    local.format("%H:%M:%S").to_string()
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes + 1023) / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

fn draw_section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().size(14.0));
    ui.add_space(2.0);
}

fn draw_directory_table(ui: &mut egui::Ui, dirs: &[String]) {
    let width = ui.available_width();
    let header_h = 24.0;
    let row_h = 25.0;
    let height = header_h + row_h * dirs.len().max(2) as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    let widths = [56.0, width - 56.0];

    painter.rect_filled(rect, 0.0, Color32::WHITE);
    painter.rect_filled(Rect::from_min_size(rect.min, Vec2::new(width, header_h)), 0.0, HEADER_BG);
    draw_text_clipped(&painter, Rect::from_min_size(rect.min, Vec2::new(widths[0], header_h)), "序号", FontId::proportional(13.0), Color32::from_rgb(35, 40, 48), true);
    draw_text_clipped(&painter, Rect::from_min_size(Pos2::new(rect.left() + widths[0], rect.top()), Vec2::new(widths[1], header_h)), "监听目录", FontId::proportional(13.0), Color32::from_rgb(35, 40, 48), true);

    for (index, dir) in dirs.iter().enumerate() {
        let y = rect.top() + header_h + row_h * index as f32;
        let row_bg = if index % 2 == 0 { Color32::WHITE } else { ROW_ALT_BG };
        painter.rect_filled(Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(width, row_h)), 0.0, row_bg);
        draw_text_clipped(&painter, Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(widths[0], row_h)), &(index + 1).to_string(), FontId::proportional(13.0), Color32::from_rgb(38, 38, 38), false);
        draw_text_clipped(&painter, Rect::from_min_size(Pos2::new(rect.left() + widths[0], y), Vec2::new(widths[1], row_h)), dir, FontId::proportional(13.0), Color32::from_rgb(38, 38, 38), false);
    }

    painter.line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(1.0_f32, TABLE_BORDER));
    painter.line_segment([Pos2::new(rect.left() + widths[0], rect.top()), Pos2::new(rect.left() + widths[0], rect.bottom())], Stroke::new(1.0_f32, GRID_LINE));
    painter.line_segment([rect.right_top(), rect.right_bottom()], Stroke::new(1.0_f32, TABLE_BORDER));
    let row_count = dirs.len().max(2);
    for index in 0..=row_count {
        let y = rect.top() + header_h + row_h * index as f32;
        if y <= rect.bottom() {
            let stroke = if index == 0 { Stroke::new(1.0_f32, TABLE_BORDER) } else { Stroke::new(1.0_f32, GRID_LINE) };
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
        }
    }
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, TABLE_BORDER));
}

fn draw_detail_table(ui: &mut egui::Ui, record: &ParseRecord) {
    let rows = [
        ("文件名", record.file_name.as_str()),
        ("完整路径", record.file_path.as_str()),
        ("文件大小", record.file_size.as_str()),
        ("修改时间", record.modified_at.as_str()),
        ("检测时间", record.detected_at.as_str()),
        ("解析时间", record.parsed_at.as_str()),
        ("插件", &format!("{} / {}", record.parser_id, record.transformer_id)),
        ("状态", record.status.label()),
        ("错误信息", if record.error_message.is_empty() { "-" } else { record.error_message.as_str() }),
    ];

    let width = ui.available_width();
    let label_w = 110.0;
    let row_h = 27.0;
    let height = row_h * rows.len() as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);

    painter.rect_filled(rect, 0.0, Color32::WHITE);
    for (index, (label, value)) in rows.iter().enumerate() {
        let y = rect.top() + row_h * index as f32;
        let row_rect = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(width, row_h));
        let label_rect = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(label_w, row_h));
        let value_rect = Rect::from_min_size(Pos2::new(rect.left() + label_w, y), Vec2::new(width - label_w, row_h));
        let row_bg = if index % 2 == 0 { Color32::WHITE } else { ROW_ALT_BG };
        painter.rect_filled(row_rect, 0.0, row_bg);
        painter.rect_filled(label_rect, 0.0, HEADER_BG);
        let value_color = if *label == "状态" { status_color(record.status) } else { Color32::from_rgb(38, 38, 38) };
        draw_text_clipped(&painter, label_rect, label, FontId::proportional(13.0), Color32::from_rgb(35, 40, 48), true);
        draw_text_clipped(&painter, value_rect, value, FontId::proportional(13.0), value_color, *label == "状态");
        painter.line_segment([Pos2::new(rect.left(), y + row_h), Pos2::new(rect.right(), y + row_h)], Stroke::new(1.0_f32, GRID_LINE));
    }

    painter.line_segment([Pos2::new(rect.left() + label_w, rect.top()), Pos2::new(rect.left() + label_w, rect.bottom())], Stroke::new(1.0_f32, GRID_LINE));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, TABLE_BORDER));
}

fn table_cell(table_rect: Rect, widths: &[f32; 7], column: usize, y: f32, height: f32) -> Rect {
    let x = table_rect.left() + widths[..column].iter().sum::<f32>();
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(widths[column], height))
}

fn draw_table_grid(
    painter: &egui::Painter,
    rect: Rect,
    widths: &[f32; 7],
    header_h: f32,
    row_h: f32,
    table_h: f32,
) {
    let mut x = rect.left();
    for width in widths {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0_f32, GRID_LINE),
        );
        x += width;
    }
    painter.line_segment(
        [Pos2::new(rect.right(), rect.top()), Pos2::new(rect.right(), rect.bottom())],
        Stroke::new(1.0_f32, TABLE_BORDER),
    );

    let row_count = ((table_h - header_h) / row_h).floor() as usize;
    for index in 0..=row_count {
        let y = rect.top() + header_h + row_h * index as f32;
        if y <= rect.bottom() {
            let stroke = if index == 0 {
                Stroke::new(1.0_f32, TABLE_BORDER)
            } else {
                Stroke::new(1.0_f32, GRID_LINE)
            };
            painter.line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
        }
    }

    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, TABLE_BORDER));
}

fn draw_text_clipped(
    painter: &egui::Painter,
    cell: Rect,
    text: &str,
    font: FontId,
    color: Color32,
    strong: bool,
) {
    let clip_rect = cell.shrink2(Vec2::new(8.0, 2.0));
    let painter = painter.with_clip_rect(clip_rect);
    let font = if strong { FontId::new(font.size + 0.5, font.family) } else { font };
    painter.text(
        Pos2::new(cell.left() + 8.0, cell.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
}

fn status_color(status: ParseStatus) -> Color32 {
    match status {
        ParseStatus::Waiting => Color32::from_rgb(95, 95, 95),
        ParseStatus::Parsing => ACCENT,
        ParseStatus::Success => Color32::from_rgb(0, 130, 80),
        ParseStatus::Failed => Color32::from_rgb(200, 40, 45),
        ParseStatus::Ignored => Color32::from_rgb(120, 120, 120),
    }
}

fn install_chinese_font(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ];

    let Some(font_bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };

    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("system_cjk".to_owned(), FontData::from_owned(font_bytes));
    fonts.families.entry(FontFamily::Proportional).or_default().insert(0, "system_cjk".to_owned());
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "system_cjk".to_owned());
    ctx.set_fonts(fonts);
}
