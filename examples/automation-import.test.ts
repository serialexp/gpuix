import { describe, expect, it } from "vitest"
import { connectTest } from "@gpuix/react/automation"

describe("linked automation package", () => {
  it("loads its transitive protocol schemas under Vitest", () => {
    expect(typeof connectTest).toBe("function")
  })
})
