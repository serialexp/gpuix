import { fileURLToPath } from "node:url"
import { defineConfig, mergeConfig } from "vitest/config"
import shared from "../../vitest.shared"

export default mergeConfig(shared, defineConfig({
  resolve: {
    // Source, as `gpx` embeds it (`packages/native/embedded/build.ts`), so
    // an app the parity tests run in both shares the tests' reconciler.
    alias: [
      {
        find: /^@gpuix\/react$/,
        replacement: fileURLToPath(new URL("./src/index.ts", import.meta.url)),
      },
    ],
  },
  test: {
    env: {
      NODE_ENV: "development",
    },
    fileParallelism: process.platform !== "linux",
  },
}))
