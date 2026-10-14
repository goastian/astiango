import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  server: { fs: { allow: [".", "../../packages/i18n"] }, proxy: { "/api": "http://127.0.0.1:3101" } },
  // The operational server and loaded tabs stay up during a frontend build.
  // Keep their hashed assets and atomically published live JSON available.
  build: { target: "es2022", emptyOutDir: false },
});
