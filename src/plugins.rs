use crate::parser::{parse_docx, ParsedDocument};
use std::path::Path;

pub trait ParserPlugin {
    fn id(&self) -> &'static str;
    fn parse(&self, path: &Path, transformer_id: &str) -> Result<ParsedDocument, String>;
}

pub struct WordDefaultParser;

impl ParserPlugin for WordDefaultParser {
    fn id(&self) -> &'static str {
        "word_default"
    }

    fn parse(&self, path: &Path, transformer_id: &str) -> Result<ParsedDocument, String> {
        parse_docx(path, self.id(), transformer_id)
    }
}

pub fn parse_with_builtin(parser_id: &str, transformer_id: &str, path: &Path) -> Result<ParsedDocument, String> {
    match parser_id {
        "word_default" => WordDefaultParser.parse(path, transformer_id),
        other => Err(format!("未找到内置解析插件：{other}")),
    }
}
