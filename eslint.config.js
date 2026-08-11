import js from '@eslint/js';
import ts from 'typescript-eslint';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';

export default ts.config(
  { ignores: ['dist/', 'node_modules/', 'vendor/', 'coverage/'] },
  js.configs.recommended,
  ...ts.configs.recommended,
  ...svelte.configs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser, ...globals.node },
    },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: {
      parserOptions: { parser: ts.parser },
    },
  },
  {
    // typescript-eslint's eslint-recommended layer is scoped to **/*.{ts,tsx,mts,cts},
    // so it never reaches .svelte. That layer both disables the core rules the compiler
    // already enforces and enables four others, so without it components run 18 rules
    // wrongly on -- including no-undef, which cannot see ambient types like NodeJS -- and
    // four wrongly off. Every component here is lang="ts", so the compiler-checks-this
    // rationale holds. Re-apply it; a plain <script> component would need reconsidering.
    files: ['**/*.svelte', '**/*.svelte.ts'],
    rules: ts.configs.eslintRecommended.rules ?? {},
  },
);
