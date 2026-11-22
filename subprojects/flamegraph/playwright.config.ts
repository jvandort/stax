import { defineConfig } from "@playwright/test"

// The browser tests, which Gradle's playwrightTest task runs with what they need.
export default defineConfig({
    testDir: "src/browserTest/ts",
    testMatch: "**/*.test.ts",
    forbidOnly: true,
    use: {
        browserName: "chromium",
        headless: true,
    },
})
