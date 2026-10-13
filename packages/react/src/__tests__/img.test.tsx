/// Tests for GPUIX custom <img> element — validates native image rendering
/// via the custom-element pipeline and visual screenshot behavior.

import fs from "fs"
import { beforeEach, describe, expect, it } from "vitest"
import React, { useLayoutEffect, useRef, useState } from "react"
import { createTestRoot, hasNativeTestRenderer, type TestRoot } from "../testing"
import type { ImgInstance } from "../types/host.js"
import { bufferSimilarity, isCI, SHOTS_DIR } from "./test-utils"

function rgbaFill(
  width: number,
  height: number,
  r: number,
  g: number,
  b: number,
  a = 255
): Buffer {
  const bytes = Buffer.alloc(width * height * 4)
  for (let i = 0; i < width * height; i++) {
    bytes[i * 4] = r
    bytes[i * 4 + 1] = g
    bytes[i * 4 + 2] = b
    bytes[i * 4 + 3] = a
  }
  return bytes
}

const describeNative = hasNativeTestRenderer ? describe : describe.skip

const IMAGE_FIXTURE_PATH = `${SHOTS_DIR}/gpuix-img-fixture.svg`
const SVG_FIXTURE = [
  '<svg xmlns="http://www.w3.org/2000/svg" width="240" height="140" viewBox="0 0 240 140">',
  '<rect x="0" y="0" width="240" height="140" fill="#1e2d59"/>',
  '<rect x="16" y="16" width="208" height="108" rx="14" fill="#5ca9ff"/>',
  '<circle cx="68" cy="70" r="24" fill="#ffd166"/>',
  '<rect x="112" y="50" width="88" height="14" rx="7" fill="#20304f"/>',
  '<rect x="112" y="74" width="70" height="12" rx="6" fill="#2a3c61"/>',
  "</svg>",
].join("")
const SVG_DATA_URL = `data:image/svg+xml;base64,${Buffer.from(SVG_FIXTURE).toString("base64")}`

function writeSvgFixture(filePath: string): void {
  fs.writeFileSync(filePath, SVG_FIXTURE, "utf8")
}

function imgBounds(renderer: TestRoot["renderer"], testId: string) {
  const element = renderer.findByTestId(testId)
  expect(element, `missing testId ${testId}`).toBeDefined()
  const rect = renderer.getElementBounds(element!.id)
  expect(rect, `no painted bounds for ${testId}`).toEqual(
    expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) })
  )
  return rect!
}

describeNative("custom element: img", () => {
  let testRoot: TestRoot

  beforeEach(() => {
    writeSvgFixture(IMAGE_FIXTURE_PATH)
    testRoot = createTestRoot()
  })

  describe("rendering", () => {
    it("should create img element and forward src/objectFit props", () => {
      function App() {
        return (
          <div style={{ width: 400, height: 240 }}>
            <img
              src={IMAGE_FIXTURE_PATH}
              objectFit="cover"
              style={{ width: 220, height: 120 }}
            />
          </div>
        )
      }

      testRoot.render(<App />)

      const images = testRoot.renderer.findByType("img")
      expect(images.length).toBe(1)
      const image = images[0] as any
      expect(image.customProps?.src).toBe(IMAGE_FIXTURE_PATH)
      expect(image.customProps?.objectFit).toBe("cover")
    })

    it("keeps a sized http src box stable across load", () => {
      testRoot.render(
        <div style={{ width: 400, height: 240, padding: 16 }}>
          <img
            testId="remote"
            src="https://example.test/gpuix-img.svg"
            objectFit="cover"
            style={{ width: 220, height: 120 }}
          />
        </div>,
      )

      const first = imgBounds(testRoot.renderer, "remote")
      expect(first.width).toBeCloseTo(220, 0)
      expect(first.height).toBeCloseTo(120, 0)

      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.flush()

      const after = imgBounds(testRoot.renderer, "remote")
      expect(after.width).toBeCloseTo(220, 0)
      expect(after.height).toBeCloseTo(120, 0)
      expect(after.x).toBeCloseTo(first.x, 0)
      expect(after.y).toBeCloseTo(first.y, 0)
    })

    it("forwards borderRadius through style", () => {
      testRoot.render(
        <img
          src={IMAGE_FIXTURE_PATH}
          objectFit="cover"
          style={{ width: 80, height: 80, borderRadius: 40 }}
        />,
      )

      const images = testRoot.renderer.findByType("img")
      expect(images.length).toBe(1)
      expect((images[0] as any).style?.borderRadius).toBe(40)
    })

  })

  describe("screenshots", () => {
    it("clips pixels to borderRadius", () => {
      function App({ radius }: { radius?: number }) {
        return (
          <div style={{ width: 160, height: 160, backgroundColor: "#00ff00", padding: 20 }}>
            <img
              src={IMAGE_FIXTURE_PATH}
              objectFit="cover"
              style={{ width: 120, height: 120, borderRadius: radius }}
            />
          </div>
        )
      }

      const square = `${SHOTS_DIR}/gpuix-img-square.png`
      const circle = `${SHOTS_DIR}/gpuix-img-circle.png`
      if (fs.existsSync(square)) fs.unlinkSync(square)
      if (fs.existsSync(circle)) fs.unlinkSync(circle)

      testRoot.render(<App />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(square)

      testRoot.render(<App radius={60} />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(circle)

      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(square), fs.readFileSync(circle))
        ).toBeLessThan(0.99)
      }
    })

    it("renders http URLs like filesystem images when width and height are set", () => {
      function App({ src }: { src: string }) {
        return <img src={src} style={{ width: 240, height: 140 }} />
      }

      const pathImage = `${SHOTS_DIR}/gpuix-img-path-url.png`
      const urlImage = `${SHOTS_DIR}/gpuix-img-http-url.png`
      if (fs.existsSync(pathImage)) fs.unlinkSync(pathImage)
      if (fs.existsSync(urlImage)) fs.unlinkSync(urlImage)

      testRoot.render(<App src={IMAGE_FIXTURE_PATH} />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(pathImage)

      testRoot.render(<App src="https://example.test/gpuix-img.svg" />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(urlImage)

      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(pathImage), fs.readFileSync(urlImage))
        ).toBeGreaterThan(0.99)
      }
    })

    it("renders base64 data URLs like filesystem images", () => {
      function App({ src }: { src: string }) {
        return <img src={src} style={{ width: 240, height: 140 }} />
      }

      const pathImage = `${SHOTS_DIR}/gpuix-img-path.png`
      const dataImage = `${SHOTS_DIR}/gpuix-img-data-url.png`
      if (fs.existsSync(pathImage)) fs.unlinkSync(pathImage)
      if (fs.existsSync(dataImage)) fs.unlinkSync(dataImage)

      testRoot.render(<App src={IMAGE_FIXTURE_PATH} />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(pathImage)

      testRoot.render(<App src={SVG_DATA_URL} />)
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(dataImage)

      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(pathImage), fs.readFileSync(dataImage))
        ).toBeGreaterThan(0.99)
      }
    })

    it("should capture screenshot changes after image source is set", () => {
      function ImageScreenshotProbe() {
        const [loaded, setLoaded] = useState(false)

        return (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              width: "100%",
              height: "100%",
              backgroundColor: "#0f111a",
            }}
          >
            <div
              style={{
                width: 420,
                height: 260,
                display: "flex",
                flexDirection: "column",
                gap: 12,
                padding: 18,
                borderRadius: 16,
                backgroundColor: "#1d2135",
              }}
              onClick={() => setLoaded(true)}
            >
              <text style={{ color: "#b3bddf", fontSize: 13 }}>
                click panel to load image
              </text>
              <img
                src={loaded ? IMAGE_FIXTURE_PATH : ""}
                objectFit="cover"
                style={{ width: 300, height: 170, borderRadius: 12 }}
              />
            </div>
          </div>
        )
      }

      testRoot.render(<ImageScreenshotProbe />)

      const path0 = `${SHOTS_DIR}/gpuix-img-0.png`
      const path1 = `${SHOTS_DIR}/gpuix-img-1.png`

      if (fs.existsSync(path0)) fs.unlinkSync(path0)
      if (fs.existsSync(path1)) fs.unlinkSync(path1)

      testRoot.renderer.captureScreenshot(path0)

      // Click centered panel to set src and start image load.
      testRoot.renderer.nativeSimulateClick(640, 400)
      // Drive extra frames to allow async image decode/load before snapshot.
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(path1)

      expect(fs.existsSync(path0)).toBe(true)
      expect(fs.existsSync(path1)).toBe(true)
      expect(fs.statSync(path0).size).toBeGreaterThan(0)
      expect(fs.statSync(path1).size).toBeGreaterThan(0)

      // Skipped on CI: Metal on macOS VMs doesn't repaint between captures.
      if (!isCI) {
        const before = fs.readFileSync(path0)
        const after = fs.readFileSync(path1)
        expect(bufferSimilarity(before, after)).toBeLessThan(0.99)
      }
    })

    it("paints pixels from setImagePixels on the img ref", () => {
      function PixelImage() {
        const imgRef = useRef<ImgInstance>(null)
        useLayoutEffect(() => {
          imgRef.current?.setImagePixels(240, 140, rgbaFill(240, 140, 255, 32, 64))
        }, [])
        return (
          <img
            ref={imgRef}
            testId="pixels"
            style={{ width: 240, height: 140 }}
          />
        )
      }

      const emptyPath = `${SHOTS_DIR}/gpuix-img-pixels-empty.png`
      const filledPath = `${SHOTS_DIR}/gpuix-img-pixels-filled.png`
      if (fs.existsSync(emptyPath)) fs.unlinkSync(emptyPath)
      if (fs.existsSync(filledPath)) fs.unlinkSync(filledPath)

      testRoot.render(<img testId="empty" style={{ width: 240, height: 140 }} />)
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(emptyPath)

      testRoot.render(<PixelImage />)
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(filledPath)

      expect(fs.statSync(filledPath).size).toBeGreaterThan(0)
      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(emptyPath), fs.readFileSync(filledPath))
        ).toBeLessThan(0.99)
      }
    })

    it("replaces pixels in place without a React src", () => {
      let imgRef: ImgInstance | null = null

      testRoot.render(
        <img
          ref={(instance) => {
            imgRef = instance as ImgInstance | null
          }}
          testId="live"
          style={{ width: 240, height: 140 }}
        />,
      )
      expect(imgRef).not.toBeNull()
      expect(typeof imgRef!.setImagePixels).toBe("function")

      const redPath = `${SHOTS_DIR}/gpuix-img-pixels-red.png`
      const bluePath = `${SHOTS_DIR}/gpuix-img-pixels-blue.png`
      if (fs.existsSync(redPath)) fs.unlinkSync(redPath)
      if (fs.existsSync(bluePath)) fs.unlinkSync(bluePath)

      imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 220, 20, 20))
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(redPath)

      imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 20, 40, 220))
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(bluePath)

      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(redPath), fs.readFileSync(bluePath))
        ).toBeLessThan(0.99)
      }
    })

    it("paints bgra pixels identical to the same rgba color", () => {
      let imgRef: ImgInstance | null = null
      testRoot.render(
        <img
          ref={(instance) => {
            imgRef = instance as ImgInstance | null
          }}
          testId="format"
          style={{ width: 240, height: 140 }}
        />,
      )
      const paths = ["rgba", "bgra", "bgra-misread"].map(
        (name) => `${SHOTS_DIR}/gpuix-img-pixels-${name}.png`,
      )
      const [rgbaPath, bgraPath, misreadPath] = paths as [string, string, string]
      for (const path of paths) if (fs.existsSync(path)) fs.unlinkSync(path)

      imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 220, 120, 20))
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(rgbaPath)

      // Same color, bytes stored as B, G, R, A.
      imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 20, 120, 220), {
        format: "bgra",
      })
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(bgraPath)

      // RGBA bytes declared as BGRA must paint red and blue swapped.
      imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 220, 120, 20), {
        format: "bgra",
      })
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(misreadPath)

      const rgbaShot = fs.readFileSync(rgbaPath)
      expect(rgbaShot.equals(fs.readFileSync(bgraPath))).toBe(true)
      // A flat fill compresses to near-identical PNG bytes, so compare exactly.
      if (!isCI) {
        expect(rgbaShot.equals(fs.readFileSync(misreadPath))).toBe(false)
      }

      expect(() =>
        imgRef!.setImagePixels(240, 140, rgbaFill(240, 140, 0, 0, 0), {
          format: "argb" as "bgra",
        }),
      ).toThrowErrorMatchingInlineSnapshot(`[Error: Unknown pixel format "argb", expected "rgba" or "bgra"]`)
    })

    it("paints encoded bytes from setImage on the img ref", () => {
      let imgRef: ImgInstance | null = null
      const emptyPath = `${SHOTS_DIR}/gpuix-img-setimage-empty.png`
      const filledPath = `${SHOTS_DIR}/gpuix-img-setimage-filled.png`
      if (fs.existsSync(emptyPath)) fs.unlinkSync(emptyPath)
      if (fs.existsSync(filledPath)) fs.unlinkSync(filledPath)

      testRoot.render(
        <img
          ref={(instance) => {
            imgRef = instance as ImgInstance | null
          }}
          testId="encoded"
          style={{ width: 240, height: 140 }}
        />,
      )
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(emptyPath)

      expect(imgRef).not.toBeNull()
      imgRef!.setImage(Buffer.from(SVG_FIXTURE))
      testRoot.renderer.flush()
      testRoot.renderer.flush()
      testRoot.renderer.captureScreenshot(filledPath)

      expect(fs.statSync(filledPath).size).toBeGreaterThan(0)
      if (!isCI) {
        expect(
          bufferSimilarity(fs.readFileSync(emptyPath), fs.readFileSync(filledPath))
        ).toBeLessThan(0.99)
      }
    })
  })
})

describeNative("custom element: svg", () => {
  it("renders raw SVG source with the style color", () => {
    const testRoot = createTestRoot()

    function SvgScreenshotProbe() {
      const [loaded, setLoaded] = useState(false)

      return (
        <div
          style={{
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            width: "100%",
            height: "100%",
            backgroundColor: "#0f111a",
          }}
          onClick={() => setLoaded(true)}
        >
          <svg
            source={loaded ? SVG_FIXTURE : ""}
            style={{ width: 240, height: 140, color: "#5ca9ff" }}
          />
        </div>
      )
    }

    testRoot.render(<SvgScreenshotProbe />)

    const beforePath = `${SHOTS_DIR}/gpuix-svg-0.png`
    const afterPath = `${SHOTS_DIR}/gpuix-svg-1.png`
    if (fs.existsSync(beforePath)) fs.unlinkSync(beforePath)
    if (fs.existsSync(afterPath)) fs.unlinkSync(afterPath)

    testRoot.renderer.captureScreenshot(beforePath)
    testRoot.renderer.nativeSimulateClick(640, 400)
    testRoot.renderer.flush()
    testRoot.renderer.flush()
    testRoot.renderer.flush()
    testRoot.renderer.captureScreenshot(afterPath)

    expect(fs.existsSync(beforePath)).toBe(true)
    expect(fs.existsSync(afterPath)).toBe(true)
    if (!isCI) {
      const before = fs.readFileSync(beforePath)
      const after = fs.readFileSync(afterPath)
      expect(bufferSimilarity(before, after)).toBeLessThan(0.99)
    }
  })

  it("takes focus from a press when it has a tab stop", () => {
    const testRoot = createTestRoot()
    const focused: string[] = []

    testRoot.render(
      <div style={{ width: "100%", height: "100%", padding: 20 }}>
        <svg
          testId="icon"
          tabIndex={0}
          source={SVG_FIXTURE}
          style={{ width: 120, height: 70 }}
          onFocus={() => focused.push("icon")}
        />
      </div>
    )

    const bounds = imgBounds(testRoot.renderer, "icon")
    testRoot.renderer.nativeSimulateClick(bounds.x + 10, bounds.y + 10)
    testRoot.renderer.flush()

    expect(focused).toEqual(["icon"])
  })
})
