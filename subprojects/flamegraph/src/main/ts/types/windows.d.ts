declare global {
    interface Window {
        WASM_MODULE_PROMISE: Promise<WebAssembly.Module>
        WASM_BASE64: Promise<string>
    }
}

export {}
