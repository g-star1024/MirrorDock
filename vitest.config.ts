import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// 与 vite.config.ts 分开：测试不需要 Tauri 的固定端口与 dev server 行为。
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    // Tauri API 依赖 window.__TAURI_INTERNALS__，jsdom 里不存在；
    // 每个测试文件通过 vi.mock 提供替代实现。
    environmentOptions: {
      jsdom: {
        url: "http://localhost:1420/",
      },
    },
  },
});
