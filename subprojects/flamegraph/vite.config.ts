import type { PluginOption, UserConfig } from "vite"
import react from "@vitejs/plugin-react"
import { viteSingleFile } from "vite-plugin-singlefile"
import { run } from "vite-plugin-run"
import path from "node:path"
import fs from "node:fs"
import zlib from "node:zlib"
import * as esbuild from "esbuild"

// Inject the wasm into the header as early as possible,
// so the browser can initialize the worker as ASAP.
const wasmPreloadPlugin = (isDev: boolean): PluginOption => ({
    name: "wasm-preloader",
    transformIndexHtml(_) {
        const wasmPath = path.resolve(
            __dirname,
            "build/wasm/flamegraph_wasm_bg.wasm",
        )

        let wasmSource: string
        if (isDev) {
            // In dev, we just point to the file served by Vite via the filesystem
            wasmSource = `/@fs${wasmPath}`
        } else {
            // In production, we inline the Base64 binary
            const buffer = fs.readFileSync(wasmPath)
            wasmSource = `data:application/wasm;base64,${buffer.toString("base64")}`
        }

        return [
            {
                tag: "script",
                attrs: { type: "text/javascript" },
                children: `
                    (function() {
                        const wasmSource = "${wasmSource}";
                        const isDataUri = wasmSource.startsWith("data:");

                        // Use streaming for dev/files, fallback to ArrayBuffer for prod/data-uris
                        window.WASM_MODULE_PROMISE = isDataUri
                            ? fetch(wasmSource).then(r => r.arrayBuffer()).then(bytes => WebAssembly.compile(bytes))
                            : WebAssembly.compileStreaming(fetch(wasmSource));

                        window.WASM_MODULE_PROMISE.catch(e => console.error("WASM Preload Failed:", e));

                        // Expose raw base64 for MCP server download
                        if (isDataUri) {
                            window.WASM_BASE64 = Promise.resolve(
                                wasmSource.slice("data:application/wasm;base64,".length)
                            );
                        } else {
                            // Dev mode: fetch wasm and convert to base64
                            window.WASM_BASE64 = fetch(wasmSource)
                                .then(r => r.arrayBuffer())
                                .then(buf => {
                                    var bytes = new Uint8Array(buf), bin = "", i = 0;
                                    while (i < bytes.length) bin += String.fromCharCode(bytes[i++]);
                                    return btoa(bin);
                                });
                        }
                    })();
                `,
                injectTo: "head-prepend",
            },
        ]
    },
})

// In dev mode, inject a demo stacks file so the MCP button has something to work with.
// Produces the same DOM structure the Java wrapper creates for real embedded stacks.
const devDemoStacksPlugin = (): PluginOption => {
    const DEMO_STACKS = `\
alpha;component_one;foo;auth 10
alpha;component_one;foo;process;db_read;connection_pool;check_idle_connections;check_feature_flag 65
alpha;component_one;foo;process;db_read 150
alpha;component_one;foo;process;transform;serialization_helper;object_mapper;write_buffer 20
alpha;component_one;foo;logging 5
alpha;component_two;foo;auth 12
alpha;component_two;foo;process;db_read 40
alpha;component_two;foo;process;transform;serialization_helper;object_mapper;instrumentation.increment_counter 45
alpha;component_two;foo;process;transform 130
alpha;component_two;foo;validate 50
alpha;component_two;foo;logging 5
asset_pipeline;process_image;decode_png 80
asset_pipeline;process_image;resize 120
asset_pipeline;process_image;compress_jpeg 150
beta;foo;auth 5
beta;foo;process;db_read;query_planner;optimizer;gc.safepoint 25
beta;foo;process;db_read 10
beta;foo;logging 3
gamma;middleware;foo;auth 9
gamma;middleware;foo;process;db_read 60
gamma;middleware;foo;process;transform;serialization_helper;object_mapper;check_feature_flag 35
gamma;middleware;foo;process;transform 55
gamma;middleware;foo;logging 4
alpha;component_one;foo;auth 14
alpha;component_one;foo;process;db_read 135
alpha;component_one;foo;process;transform;serialization_helper;object_mapper;instrumentation.increment_counter 30
alpha;component_one;foo;process;transform 35
alpha;component_one;foo;logging 6
init_subsystem;load_config;parse_yaml 90
init_subsystem;connect_downstream_api 110
delta;service;foo;auth 20
delta;service;foo;process;db_read 90
delta;service;foo;process;transform 80
delta;service;foo;validate 45
delta;service;foo;logging 8
beta;foo;auth 7
beta;foo;process;db_read;connection_pool;check_idle_connections;check_feature_flag 15
beta;foo;process;db_read 15
beta;foo;logging 4
gamma;middleware;foo;auth 11
gamma;middleware;foo;process;db_read 55
gamma;middleware;foo;process;transform 65
gamma;middleware;foo;process;db_read;query_planner;optimizer;gc.safepoint 45
gamma;middleware;foo;logging 5`

    const encodedData = zlib
        .deflateRawSync(Buffer.from(DEMO_STACKS, "utf-8"))
        .toString("base64")
    const nameBase64 = Buffer.from("demo-stacks.txt").toString("base64")

    return {
        name: "dev-demo-stacks",
        // Inject the same DOM structure the Java wrapper produces:
        //   <template id="embedded-stacks-names">base64name</template>
        //   <template id="embedded-stacks-0">base64compresseddata</template>
        // Using body-prepend so the elements are parsed before the React
        // module script, avoiding any timing dependency.
        transformIndexHtml(_) {
            return [
                {
                    tag: "template",
                    attrs: { id: "embedded-stacks-names" },
                    children: nameBase64,
                    injectTo: "body-prepend",
                },
                {
                    tag: "template",
                    attrs: { id: "embedded-stacks-0" },
                    children: encodedData,
                    injectTo: "body-prepend",
                },
            ]
        },
    }
}

// Compile mcp.ts as a standalone Node.js bundle, then gzip+base64 the result
// for embedding in the HTML via a virtual module. WASM and stacks data are
// injected at download time — no duplicate copies in the HTML.
const mcpTemplatePlugin = (): PluginOption => {
    let template = ""

    const MCP_SOURCES = [
        path.resolve(__dirname, "src/main/ts/mcp.ts"),
        path.resolve(__dirname, "src/main/ts/stackGraph.ts"),
    ]

    async function compile() {
        const result = await esbuild.build({
            entryPoints: [path.resolve(__dirname, "src/main/ts/mcp.ts")],
            bundle: true,
            platform: "node",
            format: "cjs",
            minify: true,
            write: false,
            alias: {
                "@flamegraph-wasm": path.resolve(__dirname, "build/wasm"),
            },
            // wasm_bindgen emits an async init path that uses import.meta.url,
            // which is unavailable in CJS. That path is dead code — mcp.ts
            // uses initSync() instead.
            logOverride: { "empty-import-meta": "silent" },
        })

        const js = result.outputFiles[0]!.text
        const compressed = zlib.gzipSync(Buffer.from(js))
        template = compressed.toString("base64")
    }

    return {
        name: "mcp-template",
        async buildStart() {
            await compile()
        },

        configureServer(server) {
            MCP_SOURCES.forEach((f) => server.watcher.add(f))

            server.watcher.on("change", async (file) => {
                if (!MCP_SOURCES.includes(file)) {
                    return
                }
                await compile()
                const mod = server.moduleGraph.getModuleById("\0virtual:mcp-template")
                if (mod) {
                    server.moduleGraph.invalidateModule(mod)
                }
                server.hot.send({ type: "full-reload" })
            })
        },

        resolveId(id) {
            return id === "virtual:mcp-template" ? "\0virtual:mcp-template" : null
        },

        load(id) {
            return id === "\0virtual:mcp-template"
                ? `export default ${JSON.stringify(template)}`
                : null
        },
    }
}

export default ({ command }: { command: string }): UserConfig => {
    const isDev = command === "serve"
    return {
        root: "src/main/ts",
        publicDir: "../public",
        build: {
            outDir: "../../../build/vite",
            emptyOutDir: true,
            sourcemap: "hidden",
            assetsInlineLimit: 100000000, // Ensure wasm is always inlined
        },
        base: "./",
        resolve: {
            alias: {
                "@flamegraph-wasm": path.resolve(__dirname, "build/wasm"),
            },
        },
        assetsInclude: ["**/*.wasm"],
        plugins: [
            react(),
            viteSingleFile(),
            wasmPreloadPlugin(isDev),
            mcpTemplatePlugin(),
            isDev && devDemoStacksPlugin(),
            {
                name: "watch-rust-dir",
                configureServer(server) {
                    // Configure the dev server to watch the Rust directory for changes
                    server.watcher.add(path.resolve(__dirname, "src/main/rust"))
                    server.watcher.add(path.resolve(__dirname, "build/wasm"))
                },
            } satisfies PluginOption,
            isDev &&
                run({
                    silent: false,
                    input: [
                        {
                            // Trigger compilation of rust sources whenever a change is detected
                            name: "Compile Rust",
                            startup: false,
                            run: [
                                path.resolve(__dirname, "../../gradlew"),
                                ":flamegraph:compileRust",
                            ],
                            condition: (file) =>
                                file.endsWith(".rs") ||
                                file.endsWith("Cargo.toml"),
                        },
                    ],
                }),
        ].filter(Boolean) as PluginOption[],
    }
}
