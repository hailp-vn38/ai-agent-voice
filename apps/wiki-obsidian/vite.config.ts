import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

export default defineConfig({
  plugins: [vue()],
  resolve: {
    dedupe: ["vue"],
    alias: { "@lucide/vue": fileURLToPath(new URL("./node_modules/@lucide/vue/dist/esm/lucide-vue.mjs", import.meta.url)) },
  },
  test: { environment: "jsdom", include: ["src/**/*.test.ts"], server: { deps: { inline: [/@lucide\/vue/] } } },
});
