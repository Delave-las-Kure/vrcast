import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  {
    // Git worktree исполнителей (backend/frontend/qa) физически лежат внутри дерева
    // репозитория, каждый со своим node_modules и собранным кодом. Без этой записи
    // голый `eslint .` спускается внутрь них и линтит чужой сборочный вывод, а не
    // исходники этого проекта (T549: 2517 ложных ошибок на объединённом main).
    ignores: ["dist", "src-tauri/target", ".worktrees"],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    // Вспомогательные сборщики работают в Node, а не в окне браузера: там есть
    // и `process`, и `console`, и без этой оговорки правила ругаются на них как
    // на неизвестные имена.
    files: ["scripts/**/*.{js,mjs}"],
    languageOptions: { ecmaVersion: 2022, sourceType: "module", globals: globals.node },
  },
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: { ecmaVersion: 2022, globals: globals.browser },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
    },
  },
);
