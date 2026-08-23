import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Тесты интерфейса. Ядро проверяется штатными тестами Rust — здесь только показ.
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
