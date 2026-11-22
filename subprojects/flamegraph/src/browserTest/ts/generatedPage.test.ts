import { expect, test, type ConsoleMessage, type Page } from "@playwright/test"
import { pathToFileURL } from "node:url"

/**
 * The page FlamegraphGenerator wrote, embedding small-stacks.txt and cpu-sample.jfr.
 */
function testPage(): string {
    const page: string | undefined = process.env["STAX_TEST_PAGE"]
    if (!page) {
        throw new Error("The STAX_TEST_PAGE environment variable is required")
    }
    return pathToFileURL(page).href
}

/**
 * Whether any canvas on the page has something drawn on it, other than blank or black. Runs in
 * the browser.
 */
function hasDrawnCanvas(): boolean {
    const canvases: HTMLCanvasElement[] = [...document.querySelectorAll("canvas")]
    return canvases.some((canvas: HTMLCanvasElement): boolean => {
        const context: CanvasRenderingContext2D | null = canvas.getContext("2d")
        if (!context || !canvas.width || !canvas.height) {
            return false
        }
        const pixels: Uint8ClampedArray = context.getImageData(0, 0, canvas.width, canvas.height).data
        for (let i: number = 0; i < pixels.length; i += 4) {
            const [red = 0, green = 0, blue = 0, alpha = 0]: Uint8ClampedArray = pixels.subarray(i, i + 4)
            if (alpha > 0 && red + green + blue > 0) {
                return true
            }
        }
        return false
    })
}

test("renders the embedded files", async ({ page }: { page: Page }): Promise<void> => {
    const errors: string[] = []
    page.on("pageerror", (error: Error): void => {
        errors.push(error.message)
    })
    page.on("console", (message: ConsoleMessage): void => {
        if (message.type() === "error") {
            errors.push(message.text())
        }
    })

    await page.goto(testPage())

    const fileNames: string[] = ["small-stacks.txt", "cpu-sample.jfr (cpu)"]
    for (const name of fileNames) {
        await expect(page.getByRole("button", { name, exact: true })).toBeVisible()
    }
    // The flamegraph is drawn on a canvas.
    await page.waitForFunction(hasDrawnCanvas)
    expect(errors).toEqual([])
})
