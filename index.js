import init, { uploadVideo as _uploadVideo } from './wasm_vod_uploader.js'

let initialized = false

async function ensureInit() {
  if (initialized) return
  await init()
  initialized = true
}

export async function uploadVideo(file, credential, onProgress = () => {}) {
  await ensureInit()
  // 由 JS 生成标准 GMT 时间字符串传给 WASM，避免 Rust 格式差异
  const date = new Date().toUTCString()
  return _uploadVideo(file, credential, onProgress, date)
}