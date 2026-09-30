use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::{json, Value};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

#[derive(Clone, Debug)]
pub struct ParsedDocument {
    pub raw_json: Value,
    pub final_json: Value,
}

pub fn parse_docx(path: &Path, parser_id: &str, transformer_id: &str) -> Result<ParsedDocument, String> {
    let file = File::open(path).map_err(|err| format!("打开 Word 文件失败：{err}"))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("读取 Word 压缩包失败：{err}"))?;

    let document_xml = read_zip_text(&mut archive, "word/document.xml")?;
    let blocks = parse_document_blocks(&document_xml);
    let paragraphs: Vec<Value> = blocks
        .paragraphs
        .iter()
        .enumerate()
        .map(|(index, text)| json!({ "index": index + 1, "text": text }))
        .collect();
    let tables: Vec<Value> = blocks
        .tables
        .iter()
        .enumerate()
        .map(|(index, rows)| json!({ "index": index + 1, "rows": rows }))
        .collect();

    let headers = read_prefixed_xml_texts(&mut archive, "word/header")?;
    let footers = read_prefixed_xml_texts(&mut archive, "word/footer")?;
    let metadata = read_core_metadata(&mut archive).unwrap_or_else(|_| json!({}));
    let metadata_fs = std::fs::metadata(path).map_err(|err| format!("读取文件属性失败：{err}"))?;

    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    let modified_at = metadata_fs
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_default();
    let parsed_at = chrono::Local::now().to_rfc3339();

    let raw_json = json!({
        "schema_version": "1.0",
        "source_file": {
            "file_name": file_name,
            "file_path": path.display().to_string(),
            "file_size": metadata_fs.len(),
            "modified_at": modified_at
        },
        "document": {
            "metadata": metadata,
            "headers": headers,
            "footers": footers,
            "paragraphs": paragraphs,
            "tables": tables
        },
        "parser": {
            "parser_id": parser_id,
            "parsed_at": parsed_at
        }
    });

    let final_json = transform_business_document(file_name, &blocks, transformer_id)?;

    Ok(ParsedDocument { raw_json, final_json })
}

#[derive(Default)]
struct DocumentBlocks {
    paragraphs: Vec<String>,
    tables: Vec<Vec<Vec<String>>>,
}

impl DocumentBlocks {
    fn content_text(&self) -> String {
        let mut parts = self.paragraphs.clone();
        for table in &self.tables {
            for row in table {
                let row_text = row.iter().map(|cell| cell.trim()).filter(|cell| !cell.is_empty()).collect::<Vec<_>>().join(" | ");
                if !row_text.is_empty() {
                    parts.push(row_text);
                }
            }
        }
        parts.join("\n")
    }
}

fn transform_business_document(file_name: &str, blocks: &DocumentBlocks, transformer_id: &str) -> Result<Value, String> {
    let all_text = blocks.content_text();
    if all_text.contains("测试样品状态变更登记表") || file_name.contains("KET-TR-2-2") {
        transform_change_document(file_name, blocks, transformer_id)
    } else if all_text.contains("检测委托书") || file_name.contains("KET-TR-2-1") {
        transform_create_document(file_name, blocks, transformer_id)
    } else {
        Ok(json!({
            "project": transformer_id,
            "operation": "unknown",
            "document_name": file_name,
            "content_text": all_text,
            "tables": blocks.tables
        }))
    }
}

fn transform_create_document(file_name: &str, blocks: &DocumentBlocks, transformer_id: &str) -> Result<Value, String> {
    let rows = blocks.tables.first().ok_or_else(|| "委托书缺少主信息表格".to_owned())?;
    let value = |label: &str| find_labeled_value(rows, label).unwrap_or_default();
    let supplier_name = value("委托单位");
    let sample_name = value("样品名称");
    if supplier_name.is_empty() || sample_name.is_empty() {
        return Err("委托书缺少委托单位或样品名称".to_owned());
    }
    let items = selected_table_items(blocks.tables.get(5));
    let hardware = table_objects(blocks.tables.get(1), &["workshop", "workerName", "samplingName"]);
    let software = table_objects(blocks.tables.get(2), &["workshop", "jobName"]);
    let supporting = table_objects(blocks.tables.get(3), &["workshop", "jobName"]);
    let lcd = table_objects(blocks.tables.get(4), &["workshop", "jobName"]);
    Ok(json!({
        "project": transformer_id,
        "operation": "create",
        "document_name": file_name,
        "acceptWay": 1,
        "rq": selected_checkbox_value(&value("检测类型")),
        "supplierName": supplier_name,
        "mailAddress": value("委托单位地址"),
        "inspectionName": value("制造单位"),
        "inspectionAddress": value("制造单位地址"),
        "clientContactName": value("联 系 人"),
        "clientContactPhone": value("联系电话"),
        "sampleCateName": value("样品类别"),
        "ra": sample_name,
        "rb": value("规格型号"),
        "rc": value("样品数量").parse::<u64>().unwrap_or(0),
        "rd": selected_checkbox_value(&value("测试分类")),
        "testBackground": value("检测原因"),
        "re": value("方法标准"),
        "rf": value("判定依据"),
        "rg": selected_checkbox_value(&value("保密要求")),
        "rh": selected_checkbox_value(&value("报告格式")),
        "sampleHandling": selected_checkbox_value(&value("检毕样品处置")),
        "ri": selected_checkbox_value(&value("结 果交 付")),
        "rj": selected_checkbox_value(&value("评审结论")),
        "rl": selected_checkbox_value(&value("来样检查")),
        "remark": value("备注"),
        "itemList": items,
        "HardwareVersion": hardware,
        "TestingsSoftware": software,
        "SupportingTests": supporting,
        "MatchingLCD": lcd,
        "content_text": blocks.content_text()
    }))
}

fn transform_change_document(file_name: &str, blocks: &DocumentBlocks, transformer_id: &str) -> Result<Value, String> {
    let text = blocks.content_text();
    let commission_no = text.split_once("委托编号").and_then(|(_, rest)| rest.split_once("样品状态").map(|(value, _)| value)).unwrap_or_default().trim_matches(['：', ':', ' ', '\n']).to_owned();
    let status = text.rsplit("样品状态").next().and_then(|rest| rest.chars().find(|c| c.is_ascii_digit())).and_then(|c| c.to_digit(10)).unwrap_or(0);
    let hardware = blocks.tables.first().map(|table| {
        table.iter().skip(2).filter(|row| row.len() >= 4 && !row[1].trim().is_empty()).map(|row| json!({
            "workshop": row.get(1).cloned().unwrap_or_default(),
            "workerName": row.get(2).cloned().unwrap_or_default(),
            "jobName": row.get(3).cloned().unwrap_or_default(),
            "samplingName": row.get(4).cloned().unwrap_or_default()
        })).collect::<Vec<_>>()
    }).unwrap_or_default();
    let change_reason = text.split_once("修改项目").map(|(_, rest)| rest.split_once("附件1").map(|(value, _)| value).unwrap_or(rest).trim_matches(['：', ':', ' ', '\n']).trim().to_owned()).unwrap_or_default();
    if commission_no.is_empty() {
        return Err("状态变更表缺少委托编号".to_owned());
    }
    Ok(json!({
        "project": transformer_id,
        "operation": "update",
        "document_name": file_name,
        "commissionNo": commission_no,
        "acceptWay": status,
        "rm": change_reason,
        "HardwareVersion": hardware,
        "TestingsSoftware": table_objects(blocks.tables.get(1), &["workshop", "jobName"]),
        "SupportingTests": table_objects(blocks.tables.get(2), &["workshop", "jobName"]),
        "MatchingLCD": table_objects(blocks.tables.get(3), &["workshop", "jobName"]),
        "itemList": selected_table_items(blocks.tables.get(4)),
        "content_text": text
    }))
}

fn find_labeled_value(rows: &[Vec<String>], label: &str) -> Option<String> {
    rows.iter().enumerate().find_map(|(row_index, row)| row.iter().enumerate().find_map(|(index, cell)| {
        if cell.replace(' ', "").contains(&label.replace(' ', "")) {
            row.get(index + 1).cloned().filter(|value| !value.trim().is_empty()).or_else(|| rows.get(row_index + 1).and_then(|next| next.get(index)).cloned())
        } else { None }
    }))
}

fn selected_checkbox_value(value: &str) -> String {
    let normalized = value
        .replace("□√", "☑")
        .replace("□✓", "☑")
        .replace("□✔", "☑")
        .replace("☐√", "☑")
        .replace("☐✓", "☑")
        .replace("☐✔", "☑")
        .replace("▢√", "☑")
        .replace("▢✓", "☑")
        .replace("▢✔", "☑")
        .replace("□×", "☐")
        .replace("☐×", "☐")
        .replace("▢×", "☐");
    let chars: Vec<char> = normalized.chars().collect();
    let marker_positions: Vec<(usize, char)> = chars
        .iter()
        .enumerate()
        .filter_map(|(index, character)| is_checkbox_marker(*character).then_some((index, *character)))
        .collect();
    if marker_positions.is_empty() {
        return normalize_text(value);
    }

    let mut options: Vec<(bool, String)> = Vec::new();
    for (marker_index, (position, marker)) in marker_positions.iter().enumerate() {
        let mut start = position + 1;
        let mut selected = is_checked_marker(*marker);
        while start < chars.len() && is_checkbox_marker(chars[start]) {
            if is_checked_marker(chars[start]) {
                selected = true;
            }
            start += 1;
        }
        let end = marker_positions.get(marker_index + 1).map(|(next, _)| *next).unwrap_or(chars.len());
        let text = normalize_text(&chars[start..end].iter().collect::<String>());
        options.push((selected, text));
    }

    // Some Word files insert the check mark after the option text, for example
    // `□检测方处理√□委托方领回`. In that form the checked marker has no text;
    // associate it with the preceding option.
    for index in 1..options.len() {
        if options[index].1.is_empty() && is_checked_marker(marker_positions[index].1) {
            options[index - 1].0 = true;
        }
    }

    options
        .into_iter()
        .filter(|(selected, text)| *selected && !text.is_empty())
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("、")
}

fn selected_table_items(table: Option<&Vec<Vec<String>>>) -> Vec<String> {
    let Some(table) = table else { return Vec::new(); };
    let has_selection_marker = table.iter().flatten().any(|cell| cell.chars().any(is_checkbox_marker));
    if !has_selection_marker {
        return table.iter().flat_map(|row| row.iter()).map(|cell| cell.trim()).filter(|cell| !cell.is_empty()).map(str::to_owned).collect();
    }
    table.iter().map(|row| selected_checkbox_value(&row.join(" "))).filter(|item| !item.is_empty()).collect()
}

fn normalize_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_checkbox_marker(character: char) -> bool {
    matches!(character, '□' | '☐' | '▢' | '☑' | '☒' | '■' | '√' | '✓' | '✔' | '×')
}

fn is_checked_marker(character: char) -> bool {
    matches!(character, '☑' | '☒' | '■' | '√' | '✓' | '✔')
}

fn table_objects(table: Option<&Vec<Vec<String>>>, keys: &[&str]) -> Vec<Value> {
    let Some(table) = table else { return Vec::new(); };
    table.iter().skip(1).filter(|row| row.iter().any(|cell| !cell.trim().is_empty())).map(|row| {
        let mut object = serde_json::Map::new();
        for (index, key) in keys.iter().enumerate() { object.insert((*key).to_owned(), json!(row.get(index).cloned().unwrap_or_default())); }
        Value::Object(object)
    }).collect()
}

fn read_zip_text(archive: &mut ZipArchive<File>, name: &str) -> Result<String, String> {
    let mut file = archive.by_name(name).map_err(|err| format!("读取 {name} 失败：{err}"))?;
    let mut text = String::new();
    file.read_to_string(&mut text).map_err(|err| format!("解析 {name} 文本失败：{err}"))?;
    Ok(text)
}

fn read_prefixed_xml_texts(archive: &mut ZipArchive<File>, prefix: &str) -> Result<Vec<Value>, String> {
    let mut values = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|err| format!("遍历 Word 内容失败：{err}"))?;
        let name = file.name().to_owned();
        if !name.starts_with(prefix) || !name.ends_with(".xml") {
            continue;
        }
        let mut xml = String::new();
        file.read_to_string(&mut xml).map_err(|err| format!("读取 {name} 失败：{err}"))?;
        let text = parse_plain_text(&xml);
        values.push(json!({ "part": name, "text": text }));
    }
    Ok(values)
}

fn read_core_metadata(archive: &mut ZipArchive<File>) -> Result<Value, String> {
    let xml = read_zip_text(archive, "docProps/core.xml")?;
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut current = String::new();
    let mut title = String::new();
    let mut creator = String::new();
    let mut created = String::new();
    let mut modified = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => current = local_name(event.name().as_ref()),
            Ok(Event::Text(text)) => {
                let value = text.decode().map(|cow| cow.into_owned()).unwrap_or_default();
                match current.as_str() {
                    "title" => title = value,
                    "creator" => creator = value,
                    "created" => created = value,
                    "modified" => modified = value,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    Ok(json!({
        "title": title,
        "author": creator,
        "created_at": created,
        "modified_at": modified
    }))
}

fn parse_plain_text(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut text_parts = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                if let Ok(value) = text.decode() {
                    let value = value.trim();
                    if !value.is_empty() {
                        text_parts.push(value.to_owned());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    text_parts.join(" ")
}

fn parse_document_blocks(xml: &str) -> DocumentBlocks {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut blocks = DocumentBlocks::default();
    let mut in_table = false;
    let mut in_row = false;
    let mut in_cell = false;
    let mut paragraph_text = String::new();
    let mut cell_text = String::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut current_table: Vec<Vec<String>> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match local_name(event.name().as_ref()).as_str() {
                "tbl" => {
                    in_table = true;
                    current_table.clear();
                }
                "tr" => {
                    in_row = true;
                    current_row.clear();
                }
                "tc" => {
                    in_cell = true;
                    cell_text.clear();
                }
                "p" => paragraph_text.clear(),
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if let Ok(value) = text.decode() {
                    if in_table && in_cell {
                        cell_text.push_str(&value);
                    } else {
                        paragraph_text.push_str(&value);
                    }
                }
            }
            Ok(Event::Empty(event)) => {
                if local_name(event.name().as_ref()) == "sym" {
                    if let Some(marker) = symbol_marker(&event) {
                        if in_table && in_cell {
                            cell_text.push(marker);
                        } else {
                            paragraph_text.push(marker);
                        }
                    }
                }
            }
            Ok(Event::End(event)) => match local_name(event.name().as_ref()).as_str() {
                "p" => {
                    let text = paragraph_text.trim();
                    if !in_table && !text.is_empty() {
                        blocks.paragraphs.push(text.to_owned());
                    }
                    paragraph_text.clear();
                }
                "tc" => {
                    if in_cell {
                        current_row.push(cell_text.trim().to_owned());
                    }
                    cell_text.clear();
                    in_cell = false;
                }
                "tr" => {
                    if in_row && !current_row.is_empty() {
                        current_table.push(current_row.clone());
                    }
                    current_row.clear();
                    in_row = false;
                }
                "tbl" => {
                    if !current_table.is_empty() {
                        blocks.tables.push(current_table.clone());
                    }
                    current_table.clear();
                    in_table = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    blocks
}

fn local_name(name: &[u8]) -> String {
    let raw = std::str::from_utf8(name).unwrap_or_default();
    raw.rsplit(':').next().unwrap_or(raw).to_owned()
}

fn symbol_marker(event: &quick_xml::events::BytesStart<'_>) -> Option<char> {
    let code = event.attributes().flatten().find_map(|attribute| {
        (local_name(attribute.key.as_ref()) == "char").then(|| String::from_utf8_lossy(attribute.value.as_ref()).into_owned())
    })?;
    match u32::from_str_radix(code.trim(), 16).ok()? {
        0x52 => Some('☑'),
        0x53 => Some('☐'),
        0xA3 => Some('□'),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_document_maps_commission_and_status() {
        let blocks = DocumentBlocks {
            paragraphs: vec!["测试样品状态变更登记表".to_owned(), "委托编号：KET-1".to_owned(), "样品状态:2".to_owned(), "修改项目：版本更新".to_owned()],
            tables: vec![vec![vec!["单板版本信息".to_owned()], vec!["".to_owned(), "单板名称".to_owned(), "变更前版本".to_owned(), "变更后版本".to_owned(), "芯片".to_owned()], vec!["".to_owned(), "A".to_owned(), "1".to_owned(), "2".to_owned(), "C".to_owned()]]],
        };
        let value = transform_change_document("change.docx", &blocks, "kolin").unwrap();
        assert_eq!(value["operation"], "update");
        assert_eq!(value["commissionNo"], "KET-1");
        assert_eq!(value["acceptWay"], 2);
        assert_eq!(value["HardwareVersion"][0]["jobName"], "2");
    }

    #[test]
    fn create_document_maps_labeled_fields() {
        let blocks = DocumentBlocks {
            paragraphs: vec!["检测委托书".to_owned()],
            tables: vec![vec![vec!["委托单位".to_owned(), "科林".to_owned(), "样品名称".to_owned(), "设备".to_owned(), "样品数量".to_owned(), "2".to_owned()]]],
        };
        let value = transform_create_document("create.docx", &blocks, "kolin").unwrap();
        assert_eq!(value["operation"], "create");
        assert_eq!(value["supplierName"], "科林");
        assert_eq!(value["rc"], 2);
    }

    #[test]
    fn selected_checkbox_value_returns_only_checked_options() {
        assert_eq!(selected_checkbox_value("□检测方处理□委托方领回"), "");
        assert_eq!(selected_checkbox_value("√检测方处理□委托方领回"), "检测方处理");
        assert_eq!(selected_checkbox_value("□√检测方处理□委托方领回"), "检测方处理");
        assert_eq!(selected_checkbox_value("□检测方处理□√委托方领回"), "委托方领回");
        assert_eq!(selected_checkbox_value("☐检测方处理☒委托方领回"), "委托方领回");
        assert_eq!(selected_checkbox_value("普通文本"), "普通文本");
    }
}
