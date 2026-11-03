import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const src = fileURLToPath(new URL("./src", import.meta.url));

// wadspaces.com: one static page, deployed to Cloudflare Pages
// (`wrangler pages deploy dist`).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": src } },
  server: { host: "localhost", port: 8081, strictPort: true },
  preview: { host: "localhost", port: 8081, strictPort: true },
});
