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
    css: {
      // By default Vitest stubs out every `.css` import as an empty module, matched by
      // extension regardless of a `?raw` query string — a component test never needs real
      // CSS text, so this is the right default. One test (T566, `app/__tests__/styles.
      // test.ts`) DOES need the real text: it reads a specific rule out of `styles.css` to
      // check overflow protection is actually shipped, because jsdom never applies real CSS
      // and `getComputedStyle` would answer nothing useful either way. This narrows the
      // exception to exactly that one query pattern rather than turning on full CSS
      // processing (PostCSS, CSS Modules, …) for every test in the suite.
      include: [/\.css\?raw$/],
    },
  },
});
