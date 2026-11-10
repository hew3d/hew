//! The document's geo-reference: `hew.geo.*` (kernel `geo.rs`). The
//! coordinate reference system a model is placed in: data a document
//! carries and a `.skp` keeps, never a transform on geometry.

use super::{CmdError, Ctx, Handler};
use kernel::GeoReference;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.geo.get" => Some(get),
        "hew.geo.set" => Some(set),
        _ => None,
    }
}

/// The wire form of [`GeoReference`]: the same fields, text omitted when
/// empty on the way out and defaulted on the way in.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeoWire {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default)]
    easting: f64,
    #[serde(default)]
    northing: f64,
    #[serde(default)]
    height: f64,
    #[serde(default)]
    x_axis: [f64; 2],
    #[serde(default)]
    scale: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    geodetic_datum: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    vertical_datum: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    map_projection: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    map_zone: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    map_unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    map_unit_metres: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    tgl_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    calibration_file_name: String,
}

impl From<&GeoReference> for GeoWire {
    fn from(g: &GeoReference) -> GeoWire {
        GeoWire {
            name: g.name.clone(),
            description: g.description.clone(),
            easting: g.easting,
            northing: g.northing,
            height: g.height,
            x_axis: g.x_axis,
            scale: g.scale,
            geodetic_datum: g.geodetic_datum.clone(),
            vertical_datum: g.vertical_datum.clone(),
            map_projection: g.map_projection.clone(),
            map_zone: g.map_zone.clone(),
            map_unit: g.map_unit.clone(),
            map_unit_metres: g.map_unit_metres,
            tgl_id: g.tgl_id.clone(),
            calibration_file_name: g.calibration_file_name.clone(),
        }
    }
}

impl From<GeoWire> for GeoReference {
    fn from(w: GeoWire) -> GeoReference {
        GeoReference {
            name: w.name,
            description: w.description,
            easting: w.easting,
            northing: w.northing,
            height: w.height,
            x_axis: w.x_axis,
            scale: w.scale,
            geodetic_datum: w.geodetic_datum,
            vertical_datum: w.vertical_datum,
            map_projection: w.map_projection,
            map_zone: w.map_zone,
            map_unit: w.map_unit,
            map_unit_metres: w.map_unit_metres,
            tgl_id: w.tgl_id,
            calibration_file_name: w.calibration_file_name,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetParams {}

fn get(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let GetParams {} =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let geo = ctx.doc.geo_reference().map(GeoWire::from);
    Ok(serde_json::json!({ "geo": geo }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParams {
    /// The whole record, or `null` to clear it. Required: a missing key is
    /// not a request to clear.
    geo: Value,
}

fn set(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let geo = match p.geo {
        Value::Null => None,
        v => Some(
            serde_json::from_value::<GeoWire>(v)
                .map_err(|e| CmdError::Params(e.to_string()))?
                .into(),
        ),
    };
    ctx.doc.set_geo_reference(geo)?;
    Ok(serde_json::json!({}))
}
