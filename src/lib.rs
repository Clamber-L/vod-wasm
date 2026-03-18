use base64::{engine::general_purpose, Engine as _};
use hmac::{Hmac, Mac};
use js_sys::{ArrayBuffer, Promise, Uint8Array};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, RequestMode, Response};

// ─────────────────────────────────────────────
// 常量
// ─────────────────────────────────────────────

const PART_SIZE: usize = 5 * 1024 * 1024; // 5 MB

// ─────────────────────────────────────────────
// 数据结构
// ─────────────────────────────────────────────

/// Vue 侧传入的原始阿里云凭证
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    upload_auth: String,    // Base64 编码的 JSON，包含 AK/SK/Token
    upload_address: String, // Base64 编码的 JSON，包含 Endpoint/Bucket/Object
    video_id: String,
}

/// UploadAddress 解码后的内容
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UploadAddress {
    endpoint: String, // e.g. "https://oss-cn-shanghai.aliyuncs.com"
    bucket: String,
    object: String, // OSS 对象路径
}

/// UploadAuth 解码后的内容
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UploadAuth {
    access_key_id: String,
    access_key_secret: String,
    security_token: String,
}

/// 暴露给 Vue 的上传结果
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub video_id: String,
    pub success: bool,
    pub message: String,
}

// ─────────────────────────────────────────────
// 工具函数
// ─────────────────────────────────────────────

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

fn err(msg: &str) -> JsValue {
    JsValue::from_str(msg)
}

/// Base64 解码并反序列化 JSON
fn decode_b64_json<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T, JsValue> {
    let bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| err(&format!("Base64 解码失败: {}", e)))?;
    let s = String::from_utf8(bytes).map_err(|e| err(&format!("UTF-8 解码失败: {}", e)))?;
    serde_json::from_str(&s).map_err(|e| err(&format!("JSON 解析失败: {} | 原文: {}", e, s)))
}

/// 获取当前 GMT 时间字符串（用于 OSS 签名）
fn gmt_now() -> String {
    js_sys::Date::new_0()
        .to_utc_string()
        .as_string()
        .unwrap_or_default()
}

/// OSS V1 签名
/// string_to_sign = METHOD\n\nContent-Type\nDate\nx-oss-security-token:TOKEN\n/bucket/object[?sub-resource]
fn oss_sign(
    auth: &UploadAuth,
    method: &str,
    content_type: &str,
    date: &str,
    resource: &str, // "/bucket/object" 或 "/bucket/object?uploads" 等
) -> String {
    let string_to_sign = format!(
        "{}\n\n{}\n{}\nx-oss-security-token:{}\n{}",
        method, content_type, date, auth.security_token, resource
    );

    type HmacSha1 = Hmac<Sha1>;
    let mut mac =
        HmacSha1::new_from_slice(auth.access_key_secret.as_bytes()).expect("HMAC init failed");
    mac.update(string_to_sign.as_bytes());
    let sig = general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    format!("OSS {}:{}", auth.access_key_id, sig)
}

// ─────────────────────────────────────────────
// HTTP 请求封装
// ─────────────────────────────────────────────

async fn fetch(
    url: &str,
    method: &str,
    headers: &[(&str, &str)],
    body: Option<Uint8Array>,
) -> Result<Response, JsValue> {
    let opts = RequestInit::new();
    opts.set_method(method);
    opts.set_mode(RequestMode::Cors);

    if let Some(data) = body {
        opts.set_body(&data);
    }

    let h = Headers::new()?;
    for (k, v) in headers {
        h.append(k, v)?;
    }
    opts.set_headers(&h);

    let req = Request::new_with_str_and_init(url, &opts)?;
    let window = web_sys::window().ok_or_else(|| err("no window"))?;
    let resp: Response = JsFuture::from(window.fetch_with_request(&req))
        .await?
        .dyn_into()?;

    if !resp.ok() {
        let status = resp.status();
        let text = JsFuture::from(resp.text()?)
            .await?
            .as_string()
            .unwrap_or_default();
        return Err(err(&format!("HTTP {} : {}", status, text)));
    }

    Ok(resp)
}

async fn fetch_text(
    url: &str,
    method: &str,
    headers: &[(&str, &str)],
    body: Option<Uint8Array>,
) -> Result<String, JsValue> {
    let resp = fetch(url, method, headers, body).await?;
    let text = JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .unwrap_or_default();
    Ok(text)
}

// ─────────────────────────────────────────────
// OSS 分片上传三步
// ─────────────────────────────────────────────

/// 1. InitiateMultipartUpload → 返回 uploadId
async fn initiate_multipart_upload(
    addr: &UploadAddress,
    auth: &UploadAuth,
    content_type: &str,
) -> Result<String, JsValue> {
    let url = format!(
        "{}/{}?uploads",
        addr.endpoint.trim_end_matches('/'),
        addr.object
    );
    let resource = format!("/{}/{}?uploads", addr.bucket, addr.object);
    let date = gmt_now();
    let sig = oss_sign(auth, "POST", content_type, &date, &resource);

    let xml = fetch_text(
        &url,
        "POST",
        &[
            ("Content-Type", content_type),
            ("Date", &date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        None,
    )
        .await?;

    parse_xml_tag(&xml, "UploadId")
        .ok_or_else(|| err(&format!("UploadId 解析失败，响应: {}", xml)))
}

/// 2. UploadPart → 返回 ETag
async fn upload_part(
    addr: &UploadAddress,
    auth: &UploadAuth,
    upload_id: &str,
    part_number: usize,
    chunk: Uint8Array,
) -> Result<String, JsValue> {
    let url = format!(
        "{}/{}?partNumber={}&uploadId={}",
        addr.endpoint.trim_end_matches('/'),
        addr.object,
        part_number,
        upload_id
    );
    let resource = format!(
        "/{}/{}?partNumber={}&uploadId={}",
        addr.bucket, addr.object, part_number, upload_id
    );
    let date = gmt_now();
    let sig = oss_sign(auth, "PUT", "", &date, &resource);

    let resp = fetch(
        &url,
        "PUT",
        &[
            ("Date", &date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        Some(chunk),
    )
        .await?;

    let etag = resp
        .headers()
        .get("ETag")
        .map_err(|_| err("缺少 ETag 响应头"))?
        .unwrap_or_default();

    Ok(etag)
}

/// 3. CompleteMultipartUpload
async fn complete_multipart_upload(
    addr: &UploadAddress,
    auth: &UploadAuth,
    upload_id: &str,
    parts: &[(usize, String)], // (part_number, etag)
) -> Result<(), JsValue> {
    let url = format!(
        "{}/{}?uploadId={}",
        addr.endpoint.trim_end_matches('/'),
        addr.object,
        upload_id
    );
    let resource = format!("/{}/{}?uploadId={}", addr.bucket, addr.object, upload_id);
    let date = gmt_now();

    let parts_xml: String = parts
        .iter()
        .map(|(n, etag)| {
            format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
                n, etag
            )
        })
        .collect();
    let body_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><CompleteMultipartUpload>{}</CompleteMultipartUpload>"#,
        parts_xml
    );
    let body_bytes = Uint8Array::from(body_xml.as_bytes());

    let sig = oss_sign(auth, "POST", "application/xml", &date, &resource);

    fetch(
        &url,
        "POST",
        &[
            ("Content-Type", "application/xml"),
            ("Date", &date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        Some(body_bytes),
    )
        .await?;

    Ok(())
}

// ─────────────────────────────────────────────
// 简易 XML tag 解析
// ─────────────────────────────────────────────

fn parse_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

// ─────────────────────────────────────────────
// 读取 File 为 Uint8Array
// ─────────────────────────────────────────────

async fn read_file(file: &web_sys::File) -> Result<Uint8Array, JsValue> {
    let file_reader = web_sys::FileReader::new()?;
    let fr_clone = file_reader.clone();

    let promise = Promise::new(&mut |resolve, reject| {
        let fr = fr_clone.clone();
        let onload = Closure::once(Box::new(move || {
            let result = fr.result().unwrap();
            resolve.call1(&JsValue::NULL, &result).unwrap();
        }) as Box<dyn FnOnce()>);

        let onerror = Closure::once(Box::new(move || {
            reject
                .call1(&JsValue::NULL, &err("FileReader 读取失败"))
                .unwrap();
        }) as Box<dyn FnOnce()>);

        fr_clone.set_onload(Some(onload.as_ref().unchecked_ref()));
        fr_clone.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onload.forget();
        onerror.forget();
        fr_clone.read_as_array_buffer(file).unwrap();
    });

    let buffer: ArrayBuffer = JsFuture::from(promise).await?.dyn_into()?;
    Ok(Uint8Array::new(&buffer))
}

// ─────────────────────────────────────────────
// 对外暴露的接口
// ─────────────────────────────────────────────

/// 上传视频到阿里云 VOD
///
/// # 参数
/// - `file`: 浏览器 File 对象
/// - `credential_js`: `{ uploadAuth, uploadAddress, videoId }`（阿里云原始字段，camelCase）
/// - `on_progress`: `(percent: number) => void`，进度 0-100
///
/// # 返回
/// `{ videoId, success, message }`
#[wasm_bindgen(js_name = uploadVideo)]
pub async fn upload_video(
    file: web_sys::File,
    credential_js: JsValue,
    on_progress: js_sys::Function,
) -> Result<JsValue, JsValue> {
    // 1. 解析凭证
    let cred: Credential = serde_wasm_bindgen::from_value(credential_js)
        .map_err(|e| err(&format!("凭证解析失败: {}", e)))?;

    let addr: UploadAddress = decode_b64_json(&cred.upload_address)?;
    let auth: UploadAuth = decode_b64_json(&cred.upload_auth)?;

    log(&format!(
        "[VOD] 开始上传 → endpoint: {}, bucket: {}, object: {}",
        addr.endpoint, addr.bucket, addr.object
    ));

    // 2. 读取文件
    let file_data = read_file(&file).await?;
    let total_size = file_data.byte_length() as usize;
    log(&format!("[VOD] 文件大小: {} bytes", total_size));

    // 3. 确定 Content-Type
    let content_type = {
        let t = file.type_();
        if t.is_empty() {
            "video/mp4".to_string()
        } else {
            t
        }
    };

    // 4. 初始化分片上传
    let upload_id = initiate_multipart_upload(&addr, &auth, &content_type).await?;
    log(&format!("[VOD] uploadId: {}", upload_id));

    // 5. 逐片上传
    let total_parts = (total_size + PART_SIZE - 1) / PART_SIZE;
    let mut parts: Vec<(usize, String)> = Vec::with_capacity(total_parts);

    for part_number in 1..=total_parts {
        let start = (part_number - 1) * PART_SIZE;
        let end = (start + PART_SIZE).min(total_size);
        let chunk = file_data.subarray(start as u32, end as u32);

        log(&format!("[VOD] 上传分片 {}/{}", part_number, total_parts));

        let etag = upload_part(&addr, &auth, &upload_id, part_number, chunk).await?;
        parts.push((part_number, etag));

        // 进度回调
        let percent = (part_number * 100 / total_parts) as u32;
        let _ = on_progress.call1(&JsValue::NULL, &JsValue::from(percent));
    }

    // 6. 完成上传
    complete_multipart_upload(&addr, &auth, &upload_id, &parts).await?;
    log("[VOD] 上传完成！");

    let result = UploadResult {
        video_id: cred.video_id,
        success: true,
        message: "上传成功".to_string(),
    };
    Ok(serde_wasm_bindgen::to_value(&result)?)
}
