import js from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["**/node_modules/", "**/dist/", "**/target/", "**/target-*/", "**/.venv/", "tools/convert-models/out/"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
);
