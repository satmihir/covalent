import { readFileSync } from "node:fs";
import { basename } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import react from "@vitejs/plugin-react";

/** VAD_FILES are what voice mode's speech detector loads at run time: its audio worklet, the
 *  Silero model, and ONNX Runtime's WebAssembly build. */
const VAD_FILES = [
  "@ricky0123/vad-web/dist/vad.worklet.bundle.min.js",
  "@ricky0123/vad-web/dist/silero_vad_v5.onnx",
  "onnxruntime-web/dist/ort-wasm-simd-threaded.mjs",
  "onnxruntime-web/dist/ort-wasm-simd-threaded.wasm",
].map((file) => fileURLToPath(new URL(`./node_modules/${file}`, import.meta.url)));

/** Vad serves those files under /vad/, so voice mode loads nothing from another site. */
function vad(): Plugin {
  const files = new Map(VAD_FILES.map((path) => [basename(path), path]));
  return {
    name: "covalent-vad-files",
    configureServer(server) {
      server.middlewares.use("/vad", (req, res, next) => {
        const path = files.get(basename(req.url ?? ""));
        if (!path) return next();
        const type = path.endsWith(".wasm") ? "application/wasm" : path.endsWith(".onnx") ? "application/octet-stream" : "text/javascript";
        res.setHeader("content-type", type);
        res.end(readFileSync(path));
      });
    },
    generateBundle() {
      for (const [name, path] of files) {
        this.emitFile({ type: "asset", fileName: `vad/${name}`, source: readFileSync(path) });
      }
    },
  };
}

// In development the page comes from Vite and the API from the running covalent binary.
export default defineConfig({
  plugins: [react(), vad()],
  server: {
    proxy: { "/api": "http://127.0.0.1:7420" },
  },
  // ONNX Runtime loads its WebAssembly glue at run time from /vad/; pre-bundling breaks that.
  optimizeDeps: { exclude: ["onnxruntime-web"] },
});
