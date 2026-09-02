import type { MutationRenderer, NativeRenderer } from "../types/host.js"
import { containerForRenderer, unregisterEventHandlers } from "./event-registry.js"

type SnapshotValue = object | string | number | boolean | null

interface SnapshotNode {
  id: number
  type: string
  style: object | null
  content: string | null
  events: Set<string>
  parent: number | null
  firstChild: number | null
  lastChild: number | null
  previousSibling: number | null
  nextSibling: number | null
  customProps: Map<string, SnapshotValue>
}

type EncodedSnapshotNode = [
  id: number,
  type: string,
  style: object | null,
  content: string | null,
  events: string[],
  children: number[],
  customProps: Record<string, SnapshotValue>,
]

export function wrapWithSnapshots(inner: NativeRenderer): MutationRenderer {
  if (!inner.applySnapshot) {
    throw new Error("Snapshot transport requires NativeRenderer.applySnapshot")
  }

  const nodes = new Map<number, SnapshotNode>()
  let rootId: number | null = null
  let dirty = false

  const nodeFor = (id: number): SnapshotNode | undefined => nodes.get(id)

  const detach = (node: SnapshotNode): void => {
    if (node.parent === null) return
    const parent = nodeFor(node.parent)
    const previous = node.previousSibling === null ? undefined : nodeFor(node.previousSibling)
    const next = node.nextSibling === null ? undefined : nodeFor(node.nextSibling)
    if (previous) previous.nextSibling = node.nextSibling
    else if (parent) parent.firstChild = node.nextSibling
    if (next) next.previousSibling = node.previousSibling
    else if (parent) parent.lastChild = node.previousSibling
    node.parent = null
    node.previousSibling = null
    node.nextSibling = null
  }

  const destroy = (id: number): void => {
    const node = nodeFor(id)
    if (!node) return
    detach(node)
    let childId = node.firstChild
    while (childId !== null) {
      const child = nodeFor(childId)
      const next = child?.nextSibling ?? null
      destroy(childId)
      childId = next
    }
    nodes.delete(id)
    if (rootId === id) rootId = null
  }

  const encodeNode = (node: SnapshotNode): EncodedSnapshotNode => {
    const children: number[] = []
    let childId = node.firstChild
    while (childId !== null) {
      children.push(childId)
      childId = nodeFor(childId)?.nextSibling ?? null
    }
    return [
      node.id,
      node.type,
      node.style,
      node.content,
      [...node.events],
      children,
      Object.fromEntries(node.customProps),
    ]
  }

  return {
    createElement(id, elementType) {
      nodes.set(id, {
        id,
        type: elementType,
        style: null,
        content: null,
        events: new Set(),
        parent: null,
        firstChild: null,
        lastChild: null,
        previousSibling: null,
        nextSibling: null,
        customProps: new Map(),
      })
      dirty = true
    },
    destroyElement(id) {
      destroy(id)
      dirty = true
      return []
    },
    appendChild(parentId, childId) {
      const parent = nodeFor(parentId)
      const child = nodeFor(childId)
      if (!parent || !child) return
      detach(child)
      child.parent = parentId
      child.previousSibling = parent.lastChild
      if (parent.lastChild !== null) {
        const previous = nodeFor(parent.lastChild)
        if (previous) previous.nextSibling = childId
      } else {
        parent.firstChild = childId
      }
      parent.lastChild = childId
      dirty = true
    },
    insertBefore(parentId, childId, beforeId) {
      const parent = nodeFor(parentId)
      const child = nodeFor(childId)
      if (!parent || !child) return
      const before = nodeFor(beforeId)
      if (!before || before.parent !== parentId) {
        detach(child)
        child.parent = parentId
        child.previousSibling = parent.lastChild
        if (parent.lastChild !== null) {
          const previous = nodeFor(parent.lastChild)
          if (previous) previous.nextSibling = childId
        } else {
          parent.firstChild = childId
        }
        parent.lastChild = childId
        dirty = true
        return
      }
      detach(child)
      child.parent = parentId
      child.nextSibling = beforeId
      child.previousSibling = before.previousSibling
      if (before.previousSibling !== null) {
        const previous = nodeFor(before.previousSibling)
        if (previous) previous.nextSibling = childId
      } else {
        parent.firstChild = childId
      }
      before.previousSibling = childId
      dirty = true
    },
    setStyle(id, style) {
      const node = nodeFor(id)
      if (node) node.style = style
      dirty = true
    },
    setText(id, content) {
      const node = nodeFor(id)
      if (node) node.content = content
      dirty = true
    },
    setEventListener(id, eventType, hasHandler) {
      const node = nodeFor(id)
      if (node) {
        if (hasHandler) node.events.add(eventType)
        else node.events.delete(eventType)
      }
      dirty = true
    },
    setRoot(id) {
      rootId = id
      dirty = true
    },
    setCustomProp(id, key, value) {
      const node = nodeFor(id)
      if (node) {
        if (value === null) node.customProps.delete(key)
        else node.customProps.set(key, value)
      }
      dirty = true
    },
    flushMutations() {
      if (!dirty) return
      const payload = {
        rootId,
        nodes: [...nodes.values()].map(encodeNode),
      }
      const destroyedIds = inner.applySnapshot!(JSON.stringify(payload))
      const container = containerForRenderer(inner)
      if (container) {
        for (const id of destroyedIds) {
          unregisterEventHandlers(container.eventHandlers, id)
        }
      }
      dirty = false
    },
  }
}
