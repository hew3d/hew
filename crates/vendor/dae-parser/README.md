[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](./LICENSE-MIT)
[![Apache License 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](./LICENSE-APACHE)
[![docs.rs](https://docs.rs/dae-parser/badge.svg)](https://docs.rs/dae-parser)
[![crates.io](https://img.shields.io/crates/v/dae-parser.svg)](https://crates.io/crates/dae-parser)
[![Download numbers](https://img.shields.io/crates/d/dae-parser.svg)](https://crates.io/crates/dae-parser)
[![Github CI](https://github.com/digama0/dae-parser/workflows/Continuous%20integration/badge.svg)](https://github.com/digama0/dae-parser/actions)
[![Minimum rustc version](https://img.shields.io/badge/rustc-1.61.0-lightgray.svg)](#rust-version-requirements)

<!-- cargo-sync-readme start -->

# Collada parser

This is a parser for the Collada (`.dae`) format, used for interchange between 3D renderers
and games. Compared to the [`collada`](https://crates.io/crates/collada) crate,
this crate attempts to more directly represent the Collada data model, and it is also
significantly more complete. It supports both reading and writing.

## Usage

The main entry point is the [`Document`] type, which has a [`FromStr`] implementation to convert
literal strings / slices, or [`Document::from_file`] to read from a `.dae` file on disk.

Collada documents are parsed eagerly, validating everything according to the
[COLLADA schema](https://www.khronos.org/files/collada_spec_1_4.pdf).
Once parsed, the data structures (structs and enums) can be navigated directly,
as all the data structures are public, and reflect the XML schema closely.

This library implements only version 1.4.1 of the Collada spec, although it may be expanded
in the future. (Please open an issue or PR if you find anything missing from the spec,
or if you have a use case for a later version.)

```rust
use std::str::FromStr;
use dae_parser::*;

let dae_file = r##"<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <asset>
    <created>1970-01-01T00:00:00Z</created>
    <modified>1970-01-01T00:00:00Z</modified>
  </asset>
  <library_geometries>
    <geometry id="Cube-mesh" name="Cube">
      <mesh>
        <source id="Cube-mesh-positions">
          <float_array id="Cube-mesh-positions-array" count="18">
            1 1 1 1 -1 1 1 -1 -1 -1 1 1 -1 -1 1 -1 -1 -1
          </float_array>
          <technique_common>
            <accessor source="#Cube-mesh-positions-array" count="6" stride="3">
              <param name="X" type="float"/>
              <param name="Y" type="float"/>
              <param name="Z" type="float"/>
            </accessor>
          </technique_common>
        </source>
        <vertices id="Cube-mesh-vertices">
          <input semantic="POSITION" source="#Cube-mesh-positions"/>
        </vertices>
        <triangles material="Material-material" count="4">
          <input semantic="VERTEX" source="#Cube-mesh-vertices" offset="0"/>
          <p>3 1 0 1 5 2 3 4 1 1 4 5</p>
        </triangles>
      </mesh>
    </geometry>
  </library_geometries>
</COLLADA>"##;

let document = Document::from_str(dae_file).unwrap();
let cube = document.local_map::<Geometry>().unwrap().get_str("Cube-mesh").unwrap();
let sources_map = document.local_map::<Source>().unwrap();
let vertices_map = document.local_map::<Vertices>().unwrap();
// sources.get("Cube-mesh-positions").unwrap();
assert_eq!(cube.id.as_ref().unwrap(), "Cube-mesh");
let mesh = cube.element.as_mesh().unwrap();
let tris = mesh.elements[0].as_triangles().unwrap();
assert_eq!(
    tris.data.as_deref().unwrap(),
    &[3, 1, 0, 1, 5, 2, 3, 4, 1, 1, 4, 5]
);
assert_eq!(tris.inputs[0].semantic, Semantic::Vertex);
let vertices = vertices_map.get_raw(&tris.inputs[0].source).unwrap();
assert_eq!(vertices.id, "Cube-mesh-vertices");
let source = sources_map
    .get_raw(&vertices.position_input().source)
    .unwrap();
assert_eq!(source.id.as_deref(), Some("Cube-mesh-positions"));
```
## License

Licensed under either of

 * Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
 * MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

<!-- cargo-sync-readme end -->
