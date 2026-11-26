// Browser preview with mocked Tauri APIs. The real app keeps using vite.config.ts.
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const mock = (file: string) => fileURLToPath(new URL(`./mock/${file}`, import.meta.url));

export default defineConfig({
  clearScreen: false,
  resolve: {
    alias: [
      { find: /^@tauri-apps\/api\/core$/, replacement: mock("core.ts") },
      { find: /^@tauri-apps\/api\/event$/, replacement: mock("event.ts") },
      { find: /^@tauri-apps\/api\/webview$/, replacement: mock("webview.ts") },
      { find: /^@tauri-apps\/plugin-dialog$/, replacement: mock("dialog.ts") },
      { find: /^@tauri-apps\/plugin-clipboard-manager$/, replacement: mock("clipboard.ts") },
      { find: /^@tauri-apps\/plugin-opener$/, replacement: mock("opener.ts") },
    ],
  },
});
