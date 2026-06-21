//! `convert.units` + `convert.time` — unit and timezone conversion.
//!
//! Chapter Abacus (AB.2). The convert group of the pure-compute
//! utilities pack. Both tools are gated by a single capability base,
//! **`convert.units`** (the group's flagship), at the **SemiTrusted**
//! ceiling — like `calc.eval` (AB.1), these touch no network, no
//! filesystem, and no operator data, so they are safe below the
//! Trusted tier the rest of the toolkit pins to (see
//! `docs/ABACUS.md` §2).
//!
//! ## Why tools at all
//!
//! A language model guesses unit factors and timezone offsets. A
//! curated conversion table and a bundled IANA zone database make
//! the answer exact and auditable.
//!
//! ## Tool surface
//!
//! - `convert.units` — `{value: number, from: string, to: string}`
//!   → `{value, from, to, result}`. Converts within a unit family
//!   (length / mass / temperature / volume / digital storage).
//!   Cross-family conversions (length → mass) are an error.
//! - `convert.time` — `{time: string, from: string, to: string}` →
//!   `{from: {timezone, datetime}, to: {timezone, datetime}}`.
//!   Interprets a naive local datetime in the `from` IANA timezone
//!   and re-expresses it in the `to` zone.
//!
//! ## Dependency note (AB.2)
//!
//! Unit conversion is a **hand-rolled curated table** (zero deps).
//! Timezone conversion uses **`chrono-tz`** (MIT/Apache-2.0) for the
//! bundled IANA zone database — the chapter's one new dependency,
//! offline like everything else here.

use std::str::FromStr;

use async_trait::async_trait;
use chrono::{NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// The capability base gating the whole convert group (both
/// `convert.units` and `convert.time`).
const CONVERT_SCOPE: &str = "convert.units";

// =====================================================================
// convert.units
// =====================================================================

pub struct ConvertUnits {
    id: ToolId,
    schema: Value,
}

impl Default for ConvertUnits {
    fn default() -> Self {
        Self::new()
    }
}

impl ConvertUnits {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: units_schema(),
        }
    }
}

#[async_trait]
impl Tool for ConvertUnits {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "convert.units"
    }
    fn description(&self) -> &str {
        "Convert a value between units of the same family. Input: \
         `{value: number, from: string, to: string}`. Families: \
         length (mm/cm/m/km/in/ft/yd/mi/nmi), mass \
         (mg/g/kg/t/oz/lb/st), temperature (c/f/k), volume \
         (ml/l/m3/tsp/tbsp/floz/cup/pt/qt/gal), digital \
         (bit/byte/kb/mb/gb/tb/kib/mib/gib/tib). Cross-family \
         conversion is an error. Returns `{value, from, to, \
         result}`. Scope: `convert.units`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse(CONVERT_SCOPE)
            .expect("convert.units must parse — it is in KNOWN_BASES from Chapter Abacus")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let value = match required_f64(&input, "value") {
            Ok(v) => v,
            Err(e) => return failed(self.id, format!("convert.units: {e}")),
        };
        let from = match required_string(&input, "from") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("convert.units: {e}")),
        };
        let to = match required_string(&input, "to") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("convert.units: {e}")),
        };
        match convert_units(value, &from, &to) {
            Ok(result) => ToolOutcome::Completed {
                output: json!({
                    "value": value, "from": from, "to": to, "result": result,
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => failed(self.id, format!("convert.units: {e}")),
        }
    }
}

fn units_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "value": { "type": "number", "description": "The quantity to convert." },
            "from": { "type": "string", "minLength": 1, "description": "Source unit, e.g. `km`, `lb`, `c`, `gal`, `mib`." },
            "to": { "type": "string", "minLength": 1, "description": "Target unit (same family as `from`)." }
        },
        "required": ["value", "from", "to"],
        "additionalProperties": false
    })
}

// =====================================================================
// convert.time
// =====================================================================

pub struct ConvertTime {
    id: ToolId,
    schema: Value,
}

impl Default for ConvertTime {
    fn default() -> Self {
        Self::new()
    }
}

impl ConvertTime {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: time_schema(),
        }
    }
}

#[async_trait]
impl Tool for ConvertTime {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "convert.time"
    }
    fn description(&self) -> &str {
        "Convert a local datetime between IANA timezones. Input: \
         `{time: string (e.g. `2026-06-21T14:00` or `2026-06-21 \
         14:00:00`), from: string (IANA zone, e.g. \
         `America/New_York`), to: string (IANA zone, e.g. \
         `Europe/Berlin`)}`. The `time` is read as a naive local \
         time in the `from` zone. Returns `{from: {timezone, \
         datetime}, to: {timezone, datetime}}` with RFC 3339 \
         offsets. Scope: `convert.units`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse(CONVERT_SCOPE)
            .expect("convert.units must parse — it is in KNOWN_BASES from Chapter Abacus")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let time = match required_string(&input, "time") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("convert.time: {e}")),
        };
        let from = match required_string(&input, "from") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("convert.time: {e}")),
        };
        let to = match required_string(&input, "to") {
            Ok(s) => s,
            Err(e) => return failed(self.id, format!("convert.time: {e}")),
        };
        match convert_time(&time, &from, &to) {
            Ok((from_str, to_str)) => ToolOutcome::Completed {
                output: json!({
                    "from": { "timezone": from, "datetime": from_str },
                    "to": { "timezone": to, "datetime": to_str },
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => failed(self.id, format!("convert.time: {e}")),
        }
    }
}

fn time_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "time": { "type": "string", "minLength": 1, "description": "Naive local datetime, e.g. `2026-06-21T14:00` or `2026-06-21 14:00:00`." },
            "from": { "type": "string", "minLength": 1, "description": "Source IANA timezone, e.g. `America/New_York`." },
            "to": { "type": "string", "minLength": 1, "description": "Target IANA timezone, e.g. `Europe/Berlin`." }
        },
        "required": ["time", "from", "to"],
        "additionalProperties": false
    })
}

// =====================================================================
// Pure conversion logic
// =====================================================================

/// A linear unit: family tag + multiplier to the family's base unit.
struct LinearUnit {
    family: &'static str,
    to_base: f64,
}

/// Look up a linear (non-temperature) unit by its normalized name.
fn linear_unit(name: &str) -> Option<LinearUnit> {
    // (name, family, factor-to-base). Base units: length=metre,
    // mass=gram, volume=litre, digital=byte. Curated + finite (OQ-5).
    const TABLE: &[(&str, &str, f64)] = &[
        // length → metre
        ("mm", "length", 0.001),
        ("cm", "length", 0.01),
        ("m", "length", 1.0),
        ("km", "length", 1000.0),
        ("in", "length", 0.0254),
        ("ft", "length", 0.3048),
        ("yd", "length", 0.9144),
        ("mi", "length", 1609.344),
        ("nmi", "length", 1852.0),
        // mass → gram
        ("mg", "mass", 0.001),
        ("g", "mass", 1.0),
        ("kg", "mass", 1000.0),
        ("t", "mass", 1_000_000.0),
        ("oz", "mass", 28.349_523_125),
        ("lb", "mass", 453.592_37),
        ("st", "mass", 6_350.293_18),
        // volume → litre (US customary for cooking units)
        ("ml", "volume", 0.001),
        ("l", "volume", 1.0),
        ("m3", "volume", 1000.0),
        ("tsp", "volume", 0.004_928_921_593_75),
        ("tbsp", "volume", 0.014_786_764_781_25),
        ("floz", "volume", 0.029_573_529_562_5),
        ("cup", "volume", 0.236_588_236_5),
        ("pt", "volume", 0.473_176_473),
        ("qt", "volume", 0.946_352_946),
        ("gal", "volume", 3.785_411_784),
        // digital → byte (decimal kB/MB/…; binary KiB/MiB/…)
        ("bit", "digital", 0.125),
        ("byte", "digital", 1.0),
        ("b", "digital", 1.0),
        ("kb", "digital", 1_000.0),
        ("mb", "digital", 1_000_000.0),
        ("gb", "digital", 1_000_000_000.0),
        ("tb", "digital", 1_000_000_000_000.0),
        ("kib", "digital", 1024.0),
        ("mib", "digital", 1_048_576.0),
        ("gib", "digital", 1_073_741_824.0),
        ("tib", "digital", 1_099_511_627_776.0),
    ];
    let n = name.trim().to_ascii_lowercase();
    TABLE.iter().find(|(u, _, _)| *u == n).map(|(_, family, to_base)| {
        LinearUnit { family, to_base: *to_base }
    })
}

/// Canonical temperature unit name, if `name` is a temperature.
fn temperature_unit(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "c" | "celsius" => Some("c"),
        "f" | "fahrenheit" => Some("f"),
        "k" | "kelvin" => Some("k"),
        _ => None,
    }
}

/// Convert `value` from one unit to another within the same family.
pub fn convert_units(value: f64, from: &str, to: &str) -> Result<f64, String> {
    if !value.is_finite() {
        return Err("`value` must be a finite number".to_string());
    }
    // Temperature is affine, so it gets its own path.
    if let (Some(f), Some(t)) = (temperature_unit(from), temperature_unit(to)) {
        return Ok(convert_temperature(value, f, t));
    }
    if temperature_unit(from).is_some() || temperature_unit(to).is_some() {
        return Err(format!("cannot convert between `{from}` and `{to}`"));
    }
    let f = linear_unit(from).ok_or_else(|| format!("unknown unit `{from}`"))?;
    let t = linear_unit(to).ok_or_else(|| format!("unknown unit `{to}`"))?;
    if f.family != t.family {
        return Err(format!(
            "cannot convert {} (`{from}`) to {} (`{to}`)",
            f.family, t.family
        ));
    }
    Ok(value * f.to_base / t.to_base)
}

/// Affine temperature conversion via Celsius as the pivot.
fn convert_temperature(value: f64, from: &str, to: &str) -> f64 {
    let celsius = match from {
        "c" => value,
        "f" => (value - 32.0) * 5.0 / 9.0,
        "k" => value - 273.15,
        _ => unreachable!("temperature_unit gates this"),
    };
    match to {
        "c" => celsius,
        "f" => celsius * 9.0 / 5.0 + 32.0,
        "k" => celsius + 273.15,
        _ => unreachable!("temperature_unit gates this"),
    }
}

/// Parse a naive datetime accepting a few common shapes.
fn parse_naive(time: &str) -> Option<NaiveDateTime> {
    let t = time.trim();
    const FORMATS: &[&str] = &[
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ];
    for fmt in FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Some(dt);
        }
    }
    None
}

/// Convert a naive local `time` in `from` zone to the `to` zone.
/// Returns `(from_rfc3339, to_rfc3339)`.
pub fn convert_time(time: &str, from: &str, to: &str) -> Result<(String, String), String> {
    let naive = parse_naive(time).ok_or_else(|| {
        format!("could not parse `{time}` — expected e.g. `2026-06-21T14:00` or `2026-06-21 14:00:00`")
    })?;
    let from_tz = Tz::from_str(from)
        .map_err(|_| format!("unknown timezone `{from}` (use an IANA name like `Europe/Berlin`)"))?;
    let to_tz = Tz::from_str(to)
        .map_err(|_| format!("unknown timezone `{to}` (use an IANA name like `Europe/Berlin`)"))?;
    // Localize the naive time in the source zone. A DST gap yields
    // None (the wall-clock time doesn't exist); ambiguity (fall-back)
    // yields two — take the earliest deterministically.
    let from_dt = match from_tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(dt) => dt,
        chrono::LocalResult::Ambiguous(earliest, _latest) => earliest,
        chrono::LocalResult::None => {
            return Err(format!(
                "`{time}` does not exist in `{from}` (it falls in a daylight-saving gap)"
            ));
        }
    };
    let to_dt = from_dt.with_timezone(&to_tz);
    Ok((from_dt.to_rfc3339(), to_dt.to_rfc3339()))
}

// =====================================================================
// Input helpers (mirrors calc.rs / tasks.rs)
// =====================================================================

fn required_string(input: &Value, field: &str) -> Result<String, String> {
    let s = input
        .get(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("input must include a `{field}` string field"))?;
    if s.trim().is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    Ok(s.to_string())
}

fn required_f64(input: &Value, field: &str) -> Result<f64, String> {
    input
        .get(field)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("input must include a numeric `{field}` field"))
}

fn failed(id: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    // ---- units: length / mass / volume / digital ----------------

    #[test]
    fn length_conversions() {
        assert!(approx(convert_units(1.0, "km", "m").unwrap(), 1000.0));
        assert!(approx(convert_units(1.0, "mi", "km").unwrap(), 1.609344));
        assert!(approx(convert_units(12.0, "in", "ft").unwrap(), 1.0));
    }

    #[test]
    fn mass_conversions() {
        assert!(approx(convert_units(1.0, "kg", "g").unwrap(), 1000.0));
        assert!(approx(convert_units(1.0, "lb", "oz").unwrap(), 16.0));
    }

    #[test]
    fn volume_conversions() {
        assert!(approx(convert_units(1.0, "l", "ml").unwrap(), 1000.0));
        assert!(approx(convert_units(1.0, "gal", "qt").unwrap(), 4.0));
    }

    #[test]
    fn digital_conversions() {
        assert!(approx(convert_units(1.0, "kib", "byte").unwrap(), 1024.0));
        assert!(approx(convert_units(1.0, "kb", "byte").unwrap(), 1000.0));
        assert!(approx(convert_units(8.0, "bit", "byte").unwrap(), 1.0));
    }

    #[test]
    fn units_case_insensitive() {
        assert!(approx(convert_units(1.0, "KM", "M").unwrap(), 1000.0));
    }

    #[test]
    fn identity_conversion() {
        assert!(approx(convert_units(42.0, "m", "m").unwrap(), 42.0));
    }

    // ---- units: temperature (affine) ----------------------------

    #[test]
    fn temperature_conversions() {
        assert!(approx(convert_units(100.0, "c", "f").unwrap(), 212.0));
        assert!(approx(convert_units(32.0, "f", "c").unwrap(), 0.0));
        assert!(approx(convert_units(0.0, "c", "k").unwrap(), 273.15));
        assert!(approx(convert_units(-40.0, "c", "f").unwrap(), -40.0));
    }

    // ---- units: errors ------------------------------------------

    #[test]
    fn cross_family_is_rejected() {
        assert!(convert_units(1.0, "km", "kg").unwrap_err().contains("cannot convert"));
        assert!(convert_units(1.0, "c", "m").unwrap_err().contains("cannot convert"));
    }

    #[test]
    fn unknown_unit_is_rejected() {
        assert!(convert_units(1.0, "smoot", "m").unwrap_err().contains("unknown unit"));
    }

    // ---- time ---------------------------------------------------

    #[test]
    fn timezone_conversion_basic() {
        // Summer (EDT = UTC-4, CEST = UTC+2): 14:00 NY → 20:00 Berlin.
        let (from, to) =
            convert_time("2026-06-21T14:00", "America/New_York", "Europe/Berlin").unwrap();
        assert!(from.starts_with("2026-06-21T14:00:00-04:00"), "from: {from}");
        assert!(to.starts_with("2026-06-21T20:00:00+02:00"), "to: {to}");
    }

    #[test]
    fn timezone_conversion_to_utc() {
        let (_from, to) =
            convert_time("2026-01-15 09:30:00", "America/New_York", "UTC").unwrap();
        // Winter (EST = UTC-5): 09:30 → 14:30 UTC.
        assert!(to.starts_with("2026-01-15T14:30:00+00:00"), "to: {to}");
    }

    #[test]
    fn unknown_timezone_is_rejected() {
        let e = convert_time("2026-06-21T14:00", "Mars/Olympus", "UTC").unwrap_err();
        assert!(e.contains("unknown timezone"), "got: {e}");
    }

    #[test]
    fn unparseable_time_is_rejected() {
        assert!(convert_time("not a time", "UTC", "UTC").unwrap_err().contains("could not parse"));
    }

    // ---- tool wiring --------------------------------------------

    #[test]
    fn tools_metadata_is_sound() {
        let u = ConvertUnits::new();
        assert_eq!(u.name(), "convert.units");
        assert!(!u.description().is_empty());
        assert_eq!(u.input_schema()["type"], "object");
        assert_eq!(
            u.required_scope(&json!({})),
            Scope::parse("convert.units").unwrap()
        );

        let t = ConvertTime::new();
        assert_eq!(t.name(), "convert.time");
        assert!(!t.description().is_empty());
        // Both convert tools share the convert.units group base.
        assert_eq!(
            t.required_scope(&json!({})),
            Scope::parse("convert.units").unwrap()
        );
    }
}
