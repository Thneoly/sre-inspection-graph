import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 2.x 推荐配置:
// - 固定端口(1420),禁用 host(只本地访问)
// - 路径别名 @ → src
// - 不重定向 console(让 Tauri devtools 接管)
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      // tauri build 不要重复触发 vite 热更新
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
