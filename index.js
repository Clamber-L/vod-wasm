let _wasm = null

async function init() {
  if (_wasm) return _wasm
  const wasm = await import('./pkg/wasm_vod_uploader.js')
  await wasm.default() // 初始化 wasm
  _wasm = wasm
  return wasm
}

/**
 * 上传视频到阿里云 VOD
 *
 * @param {File} file - input[type=file] 拿到的 File 对象
 * @param {{ uploadAuth: string, uploadAddress: string, videoId: string }} credential - 后端返回的阿里云原始凭证
 * @param {(percent: number) => void} [onProgress] - 进度回调，0~100
 * @returns {Promise<{ videoId: string, success: boolean, message: string }>}
 *
 * @example
 * import { uploadVideo } from 'wasm-vod-uploader'
 *
 * const credential = await fetch('/api/vod/credential').then(r => r.json())
 * const result = await uploadVideo(file, credential, (p) => console.log(p + '%'))
 * console.log(result.videoId)
 */
export async function uploadVideo(file, credential, onProgress = () => {}) {
  const wasm = await init()
  return wasm.uploadVideo(file, credential, onProgress)
}
