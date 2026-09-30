use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ParseStatus {
    Waiting,
    Parsing,
    Success,
    Failed,
    Ignored,
}

impl ParseStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Waiting => "等待中",
            Self::Parsing => "解析中",
            Self::Success => "成功",
            Self::Failed => "失败",
            Self::Ignored => "已忽略",
        }
    }

    pub fn as_code(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Parsing => "parsing",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Ignored => "ignored",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "waiting" => Self::Waiting,
            "parsing" => Self::Parsing,
            "success" => Self::Success,
            "failed" => Self::Failed,
            "ignored" => Self::Ignored,
            _ => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParseRecord {
    pub id: u64,
    pub file_name: String,
    pub file_path: String,
    pub file_size: String,
    pub modified_at: String,
    pub detected_at: String,
    pub parsed_at: String,
    pub parser_id: String,
    pub transformer_id: String,
    pub status: ParseStatus,
    pub error_message: String,
    pub raw_json: String,
    pub final_json: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AppConfig {
    pub watch_dirs: Vec<String>,
    #[serde(default)]
    pub api_base_url: String,
    #[serde(default)]
    pub creator_account: String,
    #[serde(default)]
    pub basic_auth_username: String,
    #[serde(default)]
    pub basic_auth_password: String,
    pub recursive: bool,
    pub settle_seconds: u32,
    pub start_on_boot: bool,
    pub close_to_tray: bool,
    pub retention_days: u32,
    pub save_raw_json: bool,
    pub save_final_json: bool,
    pub parser_id: String,
    pub transformer_id: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            watch_dirs: Vec::new(),
            api_base_url: String::new(),
            creator_account: String::new(),
            basic_auth_username: String::new(),
            basic_auth_password: String::new(),
            recursive: true,
            settle_seconds: 3,
            start_on_boot: true,
            close_to_tray: true,
            retention_days: 1,
            save_raw_json: true,
            save_final_json: true,
            parser_id: "word_default".to_owned(),
            transformer_id: "default".to_owned(),
        }
    }
}
