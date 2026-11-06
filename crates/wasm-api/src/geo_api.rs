//! The document's geo-reference for the app's read-only display (kernel
//! `geo.rs`). Editing goes through `hew.geo.set` on the API; a second
//! `#[wasm_bindgen] impl Scene`, like `scenes_api.rs`.

use wasm_bindgen::prelude::*;

use crate::Scene;

#[wasm_bindgen]
impl Scene {
    /// The document's coordinate reference system as JSON, or `"null"`
    /// when it has none: `{name, description, easting, northing, height,
    /// xAxisAngle (radians, null when unrecorded), scale, geodeticDatum,
    /// verticalDatum, mapProjection, mapZone, mapUnit, mapUnitMetres
    /// (null when unrecorded), tglId, calibrationFileName}`.
    pub fn geo_reference_json(&self) -> String {
        match self.doc.geo_reference() {
            None => "null".to_string(),
            Some(g) => serde_json::json!({
                "name": g.name,
                "description": g.description,
                "easting": g.easting,
                "northing": g.northing,
                "height": g.height,
                "xAxisAngle": g.x_axis_angle(),
                "scale": g.scale,
                "geodeticDatum": g.geodetic_datum,
                "verticalDatum": g.vertical_datum,
                "mapProjection": g.map_projection,
                "mapZone": g.map_zone,
                "mapUnit": g.map_unit,
                "mapUnitMetres": g.map_unit_metres,
                "tglId": g.tgl_id,
                "calibrationFileName": g.calibration_file_name,
            })
            .to_string(),
        }
    }
}
