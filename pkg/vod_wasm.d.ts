/* tslint:disable */
/* eslint-disable */

/**
 * 上传视频到阿里云 VOD
 *
 * # 参数
 * - `file`: 浏览器 File 对象
 * - `credential_js`: `{ uploadAuth, uploadAddress, videoId }`（阿里云原始字段，camelCase）
 * - `on_progress`: `(percent: number) => void`，进度 0-100
 *
 * # 返回
 * `{ videoId, success, message }`
 */
export function uploadVideo(file: File, credential_js: any, on_progress: Function): Promise<any>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly uploadVideo: (a: any, b: any, c: any) => any;
    readonly wasm_bindgen__closure__destroy__h6fc8addb65c1ec98: (a: number, b: number) => void;
    readonly wasm_bindgen__closure__destroy__h21f88ab00836062a: (a: number, b: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h56607ed82a413062: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h76feffb8bf4f2241: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hdbfaa07b6168df1b: (a: number, b: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
