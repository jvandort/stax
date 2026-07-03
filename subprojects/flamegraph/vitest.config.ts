import { defineConfig } from "vitest/config"

// Standalone vitest config: takes precedence over vite.config.ts, whose
// plugins (wasm preload, single-file packaging, MCP template) are for
// building the app and are unnecessary for unit tests.
export default defineConfig({
    test: {
        include: ["src/test/ts/**/*.test.ts"],
    },
})
