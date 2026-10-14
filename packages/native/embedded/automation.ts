// Stands in for `js/automation/client.ts` in the embedded runtime. Automation
// is served over stdin or from a browser window, and the embedded runtime has
// neither, so nothing here can be reached. Leaving the client out keeps zod,
// most of the bundle, out of every app's startup.
function unavailable(): never {
  throw new Error("Automation isn't available in the embedded runtime")
}

export class App {
  constructor() {
    unavailable()
  }
}

export class InProcessBackend {
  constructor() {
    unavailable()
  }
}

export const browserRendererAsTest = unavailable
export const liveRendererAsTest = unavailable
export const serveAutomationStdio = unavailable
