import { readFile } from "node:fs/promises";
import { defineConfig, type Plugin } from "vite";
import { viteSingleFile } from "vite-plugin-singlefile";

/**
 * `import b64 from "./x.wasm?b64"` → base64 string of the file, so the wasm
 * ends up inlined in the (inline) worker bundle. Also strips wasm-bindgen's
 * default `new URL("…_bg.wasm", import.meta.url)` so no stray asset is emitted.
 */
function wasmBase64(): Plugin {
  return {
    name: "wasm-base64",
    enforce: "pre",
    async load(id) {
      if (!id.endsWith(".wasm?b64")) return;
      const file = id.slice(0, -"?b64".length);
      this.addWatchFile(file);
      const b64 = (await readFile(file)).toString("base64");
      return `export default ${JSON.stringify(b64)};`;
    },
    transform(code, id) {
      if (!/cdc_engine\.js($|\?)/.test(id)) return;
      return code.replace(/new URL\(['"]cdc_engine_bg\.wasm['"],\s*import\.meta\.url\)/, "undefined");
    },
  };
}

export default defineConfig({
  base: "./",
  plugins: [wasmBase64(), viteSingleFile()],
  worker: { format: "iife", plugins: () => [wasmBase64()] },
  build: { target: "es2022", chunkSizeWarningLimit: 2000 },
});
