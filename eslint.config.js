// ESLint 9 flat config. Frontend (src/) only — the Rust backend is linted
// separately via `cargo clippy` (see CONTRIBUTING.md).
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import prettier from "eslint-config-prettier";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // Vite's Fast Refresh needs every export in a component file to be a
      // component; constant exports (e.g. color arrays) are fine.
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // A leading underscore marks an intentionally-unused parameter
      // (destructured props, event handler signatures, etc.).
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
    },
  },
  // Must stay last: turns off any ESLint stylistic rule that would fight
  // Prettier's own formatting.
  prettier,
);
