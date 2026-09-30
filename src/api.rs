use reqwest::blocking::{multipart, Client};
use std::path::Path;
use std::time::Duration;

/// Sends the original Word file, parsed JSON, and creator account as one multipart request.
pub fn submit_document(
    base_url: &str,
    creator_account: &str,
    basic_auth_username: &str,
    basic_auth_password: &str,
    path: &Path,
    json_payload: &str,
) -> Result<(), String> {
    let url = base_url.trim();
    if url.is_empty() {
        return Ok(());
    }
    if creator_account.trim().is_empty() {
        return Err("接口已配置，但创建人账号为空".to_owned());
    }

    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("document.docx").to_owned();
    let file = std::fs::read(path).map_err(|err| format!("读取待上传文件失败：{err}"))?;
    let part = multipart::Part::bytes(file)
        .file_name(file_name)
        .mime_str("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        .map_err(|err| format!("构造文件对象失败：{err}"))?;
    let form = multipart::Form::new()
        .part("file", part)
        .text("json", json_payload.to_owned())
        .text("creatorAccount", creator_account.trim().to_owned());

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|err| format!("创建接口客户端失败：{err}"))?;
    let mut request = client.post(url).multipart(form);
    if !basic_auth_username.trim().is_empty() {
        request = request.basic_auth(basic_auth_username.trim(), Some(basic_auth_password));
    }
    let response = request.send()
        .map_err(|err| format!("调用接口失败：{err}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(format!("接口返回 HTTP {}：{}", status.as_u16(), body.trim()));
    }
    Ok(())
}
