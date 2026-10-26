import { defineConfig } from "vitest/config"

// Bun + Vite externalization can lose Zod's named ESM exports when the
// automation protocol is imported through a linked workspace package.
export default defineConfig({ ssr: { noExternal: ["zod"] } })
