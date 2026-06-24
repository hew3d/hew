#!/usr/bin/env python3
"""Blender leg of the glTF round-trip validation.

Headless: `blender --background --python scripts/blender_roundtrip.py -- \
    <in.glb> [<in.glb> ...] <out_dir>`

For each input GLB it writes two outputs into <out_dir>:

  <name>.blender.glb   — pure import → re-export (no edits). This is the
                         fidelity baseline: what Blender's own glTF I/O does to
                         a Hew export when the user merely opens and saves.
  <name>.edited.glb    — import, apply a realistic edit (move + rotate one
                         object, then apply transforms), re-export. Mimics the
                         actual "edit in Blender" workflow targets.

Requires numpy in Blender's Python (the io_scene_gltf2 addon dependency).
"""

import bpy
import os
import sys


def _reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def _import(path):
    _reset()
    bpy.ops.import_scene.gltf(filepath=path)


def _export(path):
    bpy.ops.export_scene.gltf(filepath=path, export_format="GLB")


def passthrough(src, dst):
    _import(src)
    _export(dst)


def edited(src, dst):
    _import(src)
    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    if meshes:
        o = meshes[0]
        # A move the user might plausibly make, then bake it into the mesh so
        # the geometry itself shifts (stresses re-import, not just node poses).
        o.location = (o.location[0] + 1.0, o.location[1] + 0.5, o.location[2])
        o.rotation_euler = (0.0, 0.0, 0.7853981634)  # 45° about Z
        bpy.context.view_layer.objects.active = o
        o.select_set(True)
        bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    _export(dst)


def main():
    argv = sys.argv[sys.argv.index("--") + 1:]
    *inputs, out_dir = argv
    os.makedirs(out_dir, exist_ok=True)
    for src in inputs:
        name = os.path.splitext(os.path.basename(src))[0]
        passthrough(src, os.path.join(out_dir, f"{name}.blender.glb"))
        edited(src, os.path.join(out_dir, f"{name}.edited.glb"))
        print(f"ROUNDTRIP_DONE {name}")


if __name__ == "__main__":
    main()
