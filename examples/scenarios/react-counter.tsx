import { useState } from 'react'
import { Button, render } from '@gpuix/react'

function Counter() {
  const [count, setCount] = useState(0)
  return (
    <div style={{ padding: 24, gap: 16, width: '100%', height: '100%',
      display: 'flex', flexDirection: 'column', alignItems: 'flex-start',
      background: '#181818', color: '#e5e5e5' }}>
      <text style={{ fontSize: 20 }}>React counter</text>
      <Button testId="counter" onClick={() => setCount(value => value + 1)}
        style={{ padding: 12, borderRadius: 6, background: '#303030',
          focusVisible: { outlineWidth: 2, outlineColor: '#8ab4f8' } }}>
        <text style={{ color: '#e5e5e5', whiteSpace: 'nowrap' }}>{`Count: ${count}`}</text>
      </Button>
      <text style={{ color: '#a0a0a0' }}>Click the counter, or focus it and press Enter or Space.</text>
    </div>
  )
}

render(<Counter />, { title: 'React scenario counter' })
