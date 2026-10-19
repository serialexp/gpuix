/**
 * A chat-shaped list for the re-render benchmarks. Every row builds its `style`
 * objects and handlers from scratch on each render, the way an app that does not
 * hoist its styles does, so a parent update re-renders all of them with values
 * that are equal but never identical.
 */

import React from 'react'

export const ROWS = 1_000

function Row({ index, body }: { index: number; body: string }) {
  return (
    <div
      style={{
        flexDirection: 'column',
        gap: 6,
        padding: 12,
        borderRadius: 8,
        backgroundColor: index % 2 === 0 ? '#1e1e2e' : '#181825',
        borderWidth: 1,
        borderColor: '#313244',
        hover: { backgroundColor: '#313244' },
      }}
    >
      <div style={{ flexDirection: 'row', gap: 8, alignItems: 'center' }}>
        <div style={{ width: 20, height: 20, borderRadius: 10, backgroundColor: '#89b4fa' }} />
        <text style={{ color: '#cdd6f4', fontWeight: 600, fontSize: 13 }}>user {index}</text>
        <text style={{ color: '#6c7086', fontSize: 11 }}>12:{String(index % 60).padStart(2, '0')}</text>
      </div>
      <text style={{ color: '#cdd6f4', fontSize: 13, lineHeight: 20 }}>{body}</text>
      <div style={{ flexDirection: 'row', gap: 6 }}>
        <div
          onClick={() => {}}
          style={{ padding: 4, borderRadius: 4, backgroundColor: '#313244', hover: { backgroundColor: '#45475a' } }}
        >
          <text style={{ color: '#cdd6f4', fontSize: 11 }}>copy</text>
        </div>
        <div
          onClick={() => {}}
          style={{ padding: 4, borderRadius: 4, backgroundColor: '#313244', hover: { backgroundColor: '#45475a' } }}
        >
          <text style={{ color: '#cdd6f4', fontSize: 11 }}>retry</text>
        </div>
      </div>
    </div>
  )
}

/** `tick` changes on every update and reaches no style, so every row repeats itself. */
export function App({ tick, streaming }: { tick: number; streaming: string }) {
  return (
    <div style={{ flexDirection: 'column', width: 900, height: 700 }}>
      <text style={{ color: '#fff' }}>tick {tick}</text>
      <div style={{ flexDirection: 'column', flexGrow: 1, flexShrink: 1, flexBasis: 0, overflowY: 'scroll', gap: 8 }}>
        {Array.from({ length: ROWS }, (_, index) => (
          <Row key={index} index={index} body={index === ROWS - 1 ? streaming : `message body number ${index}`} />
        ))}
      </div>
    </div>
  )
}
