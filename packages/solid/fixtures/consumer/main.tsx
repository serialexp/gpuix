import { createTestRoot } from "@gpuix/solid/testing"
import { createView } from "./view.tsx"

const app = createTestRoot()
const fixture = createView()
app.render(fixture.view)
app.flushSync(fixture.update)
console.log(app.renderer.getAllText().join(""))
app.unmount()
