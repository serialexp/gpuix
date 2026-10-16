import type { Element } from "solid-js"
import { createRenderer } from "@solidjs/universal"
import {
  createHostElement,
  createHostText,
  getFirstChild,
  getNextSibling,
  getParentNode,
  insertHostNode,
  isHostText,
  isHostPropertyValue,
  removeHostNode,
  replaceHostText,
  setHostProperty,
  type HostNode,
  type HostParent,
} from "./host.js"

const runtime = createRenderer<HostNode | HostParent>({
  // The compiler passes props that never change here, not through setProperty.
  createElement(tag, staticProps) {
    const element = createHostElement(tag)
    if (staticProps) {
      for (const name in staticProps) {
        const value = staticProps[name]
        if (isHostPropertyValue(value)) setHostProperty(element, name, value)
      }
    }
    return element
  },
  createTextNode: createHostText,
  replaceText(node, value) {
    if (!isHostText(node)) throw new TypeError("Expected a GPUIX text node")
    replaceHostText(node, value)
  },
  setProperty(node, name, value, previous) {
    if (
      node.kind !== "root" &&
      isHostPropertyValue(value) &&
      isHostPropertyValue(previous)
    ) {
      setHostProperty(node, name, value, previous)
    }
  },
  insertNode(parent, node, anchor) {
    if (parent.kind === "text" || node.kind === "root" || anchor?.kind === "root") {
      throw new TypeError("Expected GPUIX host nodes")
    }
    insertHostNode(parent, node, anchor)
  },
  isTextNode: isHostText,
  removeNode(parent, node) {
    if (parent.kind === "text" || node.kind === "root") {
      throw new TypeError("Expected GPUIX host nodes")
    }
    removeHostNode(parent, node)
  },
  getParentNode(node) {
    return node.kind === "root" ? undefined : getParentNode(node)
  },
  getFirstChild(node) {
    return node.kind === "text" ? undefined : getFirstChild(node)
  },
  getNextSibling(node) {
    return node.kind === "root" ? undefined : getNextSibling(node)
  },
})

/** Mount `code` under `root` and flush its first render. Returns a disposer
 *  that tears the tree down and removes what it mounted. */
export function universalRender(code: () => Element, root: HostParent): () => void {
  return runtime.render(code as () => HostNode, root)
}

export const effect = runtime.effect
export const memo = runtime.memo
export const createComponent = runtime.createComponent
export const createElement = runtime.createElement
export const createTextNode = runtime.createTextNode
export const insertNode = runtime.insertNode
export const insert = runtime.insert
export const spread = runtime.spread
export const setProp = runtime.setProp
export const mergeProps = runtime.mergeProps
export const ref = runtime.ref
export const applyRef = runtime.applyRef

// The compiler imports its control flow from the renderer module.
export { Errored, For, Loading, Match, Repeat, Reveal, Show, Switch } from "solid-js"
