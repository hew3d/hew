import { useEffect, useState } from 'react'
import { loadKernel } from './wasm/loader'
import Viewport, { type MeshData } from './viewport/Viewport'

interface AppState {
  kernelVersion: string
  watertight: boolean
  meshData: MeshData
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    loadKernel()
      .then((kernel) => {
        const kernelVersion = kernel.version()
        const mesh = kernel.demo_mesh()
        try {
          const meshData: MeshData = {
            positions: mesh.positions(),
            normals: mesh.normals(),
            indices: mesh.indices(),
            edgePositions: mesh.edge_positions(),
          }
          const watertight = mesh.watertight()
          setState({ kernelVersion, watertight, meshData })
        } finally {
          mesh.free()
        }
      })
      .catch((err: unknown) => {
        setError(String(err))
      })
  }, [])

  if (error !== null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem', color: 'red' }}>
        <h1>Hew — kernel load error</h1>
        <pre>{error}</pre>
      </main>
    )
  }

  if (state === null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem' }}>
        <p>Loading kernel…</p>
      </main>
    )
  }

  return (
    <main style={{ fontFamily: 'sans-serif', padding: '1rem' }}>
      <h1>Hew — M0</h1>
      <p>Kernel version: {state.kernelVersion}</p>
      <p>Watertight: {state.watertight ? 'yes' : 'no'}</p>
      <Viewport meshData={state.meshData} />
    </main>
  )
}
