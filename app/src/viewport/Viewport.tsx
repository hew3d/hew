import { useEffect, useRef } from 'react'
import * as THREE from 'three'

export interface MeshData {
  positions: Float32Array
  normals: Float32Array
  indices: Uint32Array
  edgePositions: Float32Array
}

interface Props {
  meshData: MeshData
}

export default function Viewport({ meshData }: Props) {
  const containerRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const container = containerRef.current
    if (container === null) return

    // Renderer
    const renderer = new THREE.WebGLRenderer({ antialias: true })
    renderer.setPixelRatio(window.devicePixelRatio)
    renderer.setClearColor(0xd0d0d0)
    renderer.setSize(container.clientWidth, container.clientHeight)
    container.appendChild(renderer.domElement)

    // Scene
    const scene = new THREE.Scene()

    // Camera — placed to frame a unit tetrahedron centred near origin
    const camera = new THREE.PerspectiveCamera(
      45,
      container.clientWidth / container.clientHeight,
      0.01,
      100,
    )
    camera.position.set(2, 1.5, 2)
    camera.lookAt(0, 0.3, 0)

    // Lights
    const ambient = new THREE.AmbientLight(0xffffff, 0.4)
    scene.add(ambient)

    const dirLight = new THREE.DirectionalLight(0xffffff, 0.9)
    dirLight.position.set(3, 5, 4)
    scene.add(dirLight)

    // Face mesh
    const geometry = new THREE.BufferGeometry()
    geometry.setAttribute(
      'position',
      new THREE.BufferAttribute(meshData.positions, 3),
    )
    geometry.setAttribute(
      'normal',
      new THREE.BufferAttribute(meshData.normals, 3),
    )
    geometry.setIndex(new THREE.BufferAttribute(meshData.indices, 1))

    const material = new THREE.MeshPhongMaterial({
      color: 0xa8c8e8,
      flatShading: true,
    })
    const mesh = new THREE.Mesh(geometry, material)
    scene.add(mesh)

    // Edges (SketchUp look)
    const edgeGeometry = new THREE.BufferGeometry()
    edgeGeometry.setAttribute(
      'position',
      new THREE.BufferAttribute(meshData.edgePositions, 3),
    )
    const edgeMaterial = new THREE.LineBasicMaterial({ color: 0x1a1a1a })
    const edges = new THREE.LineSegments(edgeGeometry, edgeMaterial)
    scene.add(edges)

    // Initial render
    renderer.render(scene, camera)

    // Resize handling
    const resizeObserver = new ResizeObserver(() => {
      const w = container.clientWidth
      const h = container.clientHeight
      camera.aspect = w / h
      camera.updateProjectionMatrix()
      renderer.setSize(w, h)
      renderer.render(scene, camera)
    })
    resizeObserver.observe(container)

    return () => {
      resizeObserver.disconnect()
      renderer.dispose()
      geometry.dispose()
      material.dispose()
      edgeGeometry.dispose()
      edgeMaterial.dispose()
      container.removeChild(renderer.domElement)
    }
  }, [meshData])

  return (
    <div
      ref={containerRef}
      style={{ width: '100%', height: '480px' }}
    />
  )
}
