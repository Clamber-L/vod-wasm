use base64::{engine::general_purpose, Engine as _};
use js_sys::{ArrayBuffer, Promise, Uint8Array};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, Request, RequestInit, RequestMode, Response};

const PART_SIZE: usize = 5 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Credential {
    upload_auth: String,
    upload_address: String,
    video_id: String,
}

#[derive(Deserialize)]
struct UploadAddress {
    #[serde(rename = "Endpoint")]
    endpoint: String,
    #[serde(rename = "Bucket")]
    bucket: String,
    #[serde(rename = "FileName", alias = "Object")]
    file_name: String,
}

#[derive(Deserialize)]
struct UploadAuth {
    #[serde(rename = "AccessKeyId")]
    access_key_id: String,
    #[serde(rename = "AccessKeySecret")]
    access_key_secret: String,
    #[serde(rename = "SecurityToken")]
    security_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadResult {
    pub video_id: String,
    pub success: bool,
    pub message: String,
}

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

fn err(msg: &str) -> JsValue {
    JsValue::from_str(msg)
}

fn decode_b64_json<T: for<'de> Deserialize<'de>>(encoded: &str) -> Result<T, JsValue> {
    let bytes = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| err(&format!("Base64 解码失败: {}", e)))?;
    let s = String::from_utf8(bytes).map_err(|e| err(&format!("UTF-8 解码失败: {}", e)))?;
    serde_json::from_str(&s).map_err(|e| err(&format!("JSON 解析失败: {} | 原文: {}", e, s)))
}

fn oss_base_url(endpoint: &str, bucket: &str) -> String {
    if let Some(rest) = endpoint.trim_end_matches('/').strip_prefix("https://") {
        format!("https://{}.{}", bucket, rest)
    } else if let Some(rest) = endpoint.trim_end_matches('/').strip_prefix("http://") {
        format!("http://{}.{}", bucket, rest)
    } else {
        format!("{}/{}", endpoint.trim_end_matches('/'), bucket)
    }
}

/// 使用浏览器 crypto.subtle 计算 HMAC-SHA1，结果与 JS 完全一致
async fn oss_sign(
    auth: &UploadAuth,
    method: &str,
    content_type: &str,
    date: &str,
    resource: &str,
) -> Result<String, JsValue> {
    let string_to_sign = format!(
        "{}\n\n{}\n{}\nx-oss-date:{}\nx-oss-security-token:{}\n{}",
        method, content_type, date, date, auth.security_token, resource
    );

    let window = web_sys::window().ok_or_else(|| err("no window"))?;
    let subtle = window.crypto().map_err(|_| err("no crypto"))?.subtle();

    let key_data = Uint8Array::from(auth.access_key_secret.as_bytes());
    let algorithm = js_sys::Object::new();
    js_sys::Reflect::set(&algorithm, &"name".into(), &"HMAC".into())?;
    let hash_obj = js_sys::Object::new();
    js_sys::Reflect::set(&hash_obj, &"name".into(), &"SHA-1".into())?;
    js_sys::Reflect::set(&algorithm, &"hash".into(), &hash_obj)?;
    let usages = js_sys::Array::new();
    usages.push(&"sign".into());

    let crypto_key: web_sys::CryptoKey = JsFuture::from(
        subtle
            .import_key_with_object("raw", &key_data, &algorithm, false, &usages)
            .map_err(|e| err(&format!("import_key failed: {:?}", e)))?,
    )
        .await?
        .dyn_into()
        .map_err(|_| err("dyn_into CryptoKey failed"))?;

    let data = Uint8Array::from(string_to_sign.as_bytes());
    let sign_alg = js_sys::Object::new();
    js_sys::Reflect::set(&sign_alg, &"name".into(), &"HMAC".into())?;

    let sig_buffer: ArrayBuffer = JsFuture::from(
        subtle
            .sign_with_object_and_buffer_source(&sign_alg, &crypto_key, &data)
            .map_err(|e| err(&format!("sign failed: {:?}", e)))?,
    )
        .await?
        .dyn_into()
        .map_err(|_| err("dyn_into ArrayBuffer failed"))?;

    let sig_bytes = Uint8Array::new(&sig_buffer);
    let mut sig_vec = vec![0u8; sig_bytes.length() as usize];
    sig_bytes.copy_to(&mut sig_vec);

    Ok(format!(
        "OSS {}:{}",
        auth.access_key_id,
        general_purpose::STANDARD.encode(&sig_vec)
    ))
}

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
    Ok(JsFuture::from(resp.text()?)
        .await?
        .as_string()
        .unwrap_or_default())
}

async fn initiate_multipart_upload(
    addr: &UploadAddress,
    auth: &UploadAuth,
    content_type: &str,
    date: &str,
) -> Result<String, JsValue> {
    let base = oss_base_url(&addr.endpoint, &addr.bucket);
    let url = format!("{}/{}?uploads", base, addr.file_name);
    let resource = format!("/{}/{}?uploads", addr.bucket, addr.file_name);
    let sig = oss_sign(auth, "POST", content_type, date, &resource).await?;

    let xml = fetch_text(
        &url,
        "POST",
        &[
            ("Content-Type", content_type),
            ("Date", date),
            ("x-oss-date", date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        None,
    )
        .await?;

    parse_xml_tag(&xml, "UploadId")
        .ok_or_else(|| err(&format!("UploadId 解析失败: {}", xml)))
}

async fn upload_part(
    addr: &UploadAddress,
    auth: &UploadAuth,
    upload_id: &str,
    part_number: usize,
    chunk: Uint8Array,
    date: &str,
) -> Result<String, JsValue> {
    let base = oss_base_url(&addr.endpoint, &addr.bucket);
    let url = format!(
        "{}/{}?partNumber={}&uploadId={}",
        base, addr.file_name, part_number, upload_id
    );
    let resource = format!(
        "/{}/{}?partNumber={}&uploadId={}",
        addr.bucket, addr.file_name, part_number, upload_id
    );
    let sig = oss_sign(auth, "PUT", "", date, &resource).await?;

    let resp = fetch(
        &url,
        "PUT",
        &[
            ("Date", date),
            ("x-oss-date", date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        Some(chunk),
    )
        .await?;

    resp.headers()
        .get("ETag")
        .map_err(|_| err("缺少 ETag 响应头"))?
        .ok_or_else(|| err("ETag 为空"))
}

async fn complete_multipart_upload(
    addr: &UploadAddress,
    auth: &UploadAuth,
    upload_id: &str,
    parts: &[(usize, String)],
    date: &str,
) -> Result<(), JsValue> {
    let base = oss_base_url(&addr.endpoint, &addr.bucket);
    let url = format!("{}/{}?uploadId={}", base, addr.file_name, upload_id);
    let resource = format!("/{}/{}?uploadId={}", addr.bucket, addr.file_name, upload_id);

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
    let sig = oss_sign(auth, "POST", "application/xml", date, &resource).await?;

    fetch(
        &url,
        "POST",
        &[
            ("Content-Type", "application/xml"),
            ("Date", date),
            ("x-oss-date", date),
            ("Authorization", &sig),
            ("x-oss-security-token", &auth.security_token),
        ],
        Some(body_bytes),
    )
        .await?;

    Ok(())
}

fn parse_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

async fn read_file(file: &web_sys::File) -> Result<Uint8Array, JsValue> {
    let fr = web_sys::FileReader::new()?;
    let fr_clone = fr.clone();
    let promise = Promise::new(&mut |resolve, reject| {
        let fr2 = fr_clone.clone();
        let onload = Closure::once(Box::new(move || {
            resolve.call1(&JsValue::NULL, &fr2.result().unwrap()).unwrap();
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

#[wasm_bindgen(js_name = uploadVideo)]
pub async fn upload_video(
    file: web_sys::File,
    credential_js: JsValue,
    on_progress: js_sys::Function,
    date_str: Option<String>,
) -> Result<JsValue, JsValue> {
    let cred: Credential = serde_wasm_bindgen::from_value(credential_js)
        .map_err(|e| err(&format!("凭证解析失败: {}", e)))?;
    let addr: UploadAddress = decode_b64_json(&cred.upload_address)?;
    let auth: UploadAuth = decode_b64_json(&cred.upload_auth)?;

    let date = date_str.unwrap_or_else(|| {
        let d = js_sys::Date::new_0();
        let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
        format!(
            "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
            days[d.get_utc_day() as usize],
            d.get_utc_date(),
            months[d.get_utc_month() as usize],
            d.get_utc_full_year(),
            d.get_utc_hours(),
            d.get_utc_minutes(),
            d.get_utc_seconds(),
        )
    });

    log(&format!("[VOD] 开始上传 → bucket: {}, object: {}", addr.bucket, addr.file_name));

    let file_data = read_file(&file).await?;
    let total_size = file_data.byte_length() as usize;
    log(&format!("[VOD] 文件大小: {} bytes", total_size));

    let content_type = {
        let t = file.type_();
        if t.is_empty() { "video/mp4".to_string() } else { t }
    };

    let upload_id = initiate_multipart_upload(&addr, &auth, &content_type, &date).await?;
    log(&format!("[VOD] uploadId: {}", upload_id));

    let total_parts = (total_size + PART_SIZE - 1) / PART_SIZE;
    let mut parts: Vec<(usize, String)> = Vec::with_capacity(total_parts);

    for part_number in 1..=total_parts {
        let start = (part_number - 1) * PART_SIZE;
        let end = (start + PART_SIZE).min(total_size);
        let chunk = file_data.subarray(start as u32, end as u32);
        log(&format!("[VOD] 上传分片 {}/{}", part_number, total_parts));
        let etag = upload_part(&addr, &auth, &upload_id, part_number, chunk, &date).await?;
        parts.push((part_number, etag));
        let percent = (part_number * 100 / total_parts) as u32;
        let _ = on_progress.call1(&JsValue::NULL, &JsValue::from(percent));
    }

    complete_multipart_upload(&addr, &auth, &upload_id, &parts, &date).await?;
    log("[VOD] 上传完成！");

    Ok(serde_wasm_bindgen::to_value(&UploadResult {
        video_id: cred.video_id,
        success: true,
        message: "上传成功".to_string(),
    })?)
}