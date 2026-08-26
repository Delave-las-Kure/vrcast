import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    // **The loopback address, written out.** Left as `false` this listens on "localhost",
    // which Node resolves to `::1` first on some machines — while the application's window
    // asks for `http://localhost:1420` and its web view resolves that to `127.0.0.1`. Both
    // are behaving correctly and nothing is listening where the window looks: the window
    // comes up saying the connection was refused, with a dev server running happily beside
    // it. Naming the address leaves neither of them a choice.
    //
    // Still only the loopback: this does not put the dev server on the network. Reaching it
    // from another device is what TAURI_DEV_HOST is for, and that path is untouched.
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
