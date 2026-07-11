//! The `tswift.db.*` host-service wire: op names, stage-1 signatures, and the
//! tagged SQL-value codec.
//!
//! See `docs/adr/0015-db-host-service-wire.md` for the full wire contract and
//! the design tradeoffs behind it (mirrors `tswift.defaults.*`/`tswift.fs.*`
//! from ADR-0014). Summary:
//!
//! - Seven host functions, all declared with stage-1 types
//!   ([`tswift_core::host_bridge`]): `handle`s are `Int`; `sql` is `String`;
//!   `params` (bind values) and query results travel as a `String` — the JSON
//!   encoding of a *tagged* value document (see [`DbValue`]) — exactly like
//!   `tswift.defaults.*` smuggles a heterogeneous stored value through a
//!   `String` payload.
//! - `tswift.db.open(path: String) -> Int` — opens (creating if absent) the
//!   database at `path` and returns an opaque, process-local handle.
//! - `tswift.db.close(handle: Int) -> Void`.
//! - `tswift.db.execute(handle: Int, sql: String, params: String) -> String`
//!   — `params` is the JSON encoding of a [`DbValue`] array (positional `?`
//!   binds); the reply is a JSON object `{"rowsAffected": Int,
//!   "lastInsertRowid": Int}`.
//! - `tswift.db.query(handle: Int, sql: String, params: String) -> String` —
//!   same `params` shape; the reply is a JSON array of column-name-keyed
//!   objects, one per row, each value a tagged [`DbValue`] document.
//! - `tswift.db.begin(handle: Int) -> Void` /
//!   `tswift.db.commit(handle: Int) -> Void` /
//!   `tswift.db.rollback(handle: Int) -> Void` — a transaction is three
//!   ordinary sequential calls against the same handle, not one atomic batch
//!   op (see the ADR for why the synchronous, single-threaded host bridge
//!   makes that sound).
//! - Every function `throws`; a host-side SQL/handle failure crosses as a
//!   `{"$thrown": "<code>: <message>"}` payload (see
//!   `tswift_core::host_bridge`), which the interpreter turns into a
//!   catchable `HostError { message: String }` — the same structured
//!   code-plus-message shape `tswift.fs.*`/`tswift.defaults.*` already use.

use tswift_core::json::{self, Json};

/// `tswift.db.open(path: String) -> Int`.
pub const OP_OPEN: &str = "tswift.db.open";
/// `tswift.db.close(handle: Int) -> Void`.
pub const OP_CLOSE: &str = "tswift.db.close";
/// `tswift.db.execute(handle: Int, sql: String, params: String) -> String`.
pub const OP_EXECUTE: &str = "tswift.db.execute";
/// `tswift.db.query(handle: Int, sql: String, params: String) -> String`.
pub const OP_QUERY: &str = "tswift.db.query";
/// `tswift.db.begin(handle: Int) -> Void`.
pub const OP_BEGIN: &str = "tswift.db.begin";
/// `tswift.db.commit(handle: Int) -> Void`.
pub const OP_COMMIT: &str = "tswift.db.commit";
/// `tswift.db.rollback(handle: Int) -> Void`.
pub const OP_ROLLBACK: &str = "tswift.db.rollback";

/// The full set of `tswift.db.*` host-function signatures, in
/// [`tswift_core::host_bridge::Signature`]'s JSON schema — what
/// [`crate::install`] declares on the interpreter.
pub const HOST_FN_SIGNATURES: &[&str] = &[
    r#"{"name":"tswift.db.open","params":[{"label":"path","type":"String"}],"returns":"Int","throws":true}"#,
    r#"{"name":"tswift.db.close","params":[{"label":"handle","type":"Int"}],"returns":"Void","throws":true}"#,
    r#"{"name":"tswift.db.execute","params":[{"label":"handle","type":"Int"},{"label":"sql","type":"String"},{"label":"params","type":"String"}],"returns":"String","throws":true}"#,
    r#"{"name":"tswift.db.query","params":[{"label":"handle","type":"Int"},{"label":"sql","type":"String"},{"label":"params","type":"String"}],"returns":"String","throws":true}"#,
    r#"{"name":"tswift.db.begin","params":[{"label":"handle","type":"Int"}],"returns":"Void","throws":true}"#,
    r#"{"name":"tswift.db.commit","params":[{"label":"handle","type":"Int"}],"returns":"Void","throws":true}"#,
    r#"{"name":"tswift.db.rollback","params":[{"label":"handle","type":"Int"}],"returns":"Void","throws":true}"#,
];

/// A SQL value tagged with its SQLite storage class, so it round-trips
/// losslessly across the JSON wire — plain JSON can't tell a `REAL` `5.0`
/// apart from an `INTEGER` `5`, or represent a `BLOB` at all.
///
/// Wire encoding: a single-key-tagged JSON object, `{"<tag>": <payload>}`
/// (`null` carries no payload: bare `{"null":null}`). Tags: `null`, `int`
/// (JSON number, `i64`), `real` (JSON number, `f64`), `text` (JSON string),
/// `blob` (JSON string, base64 via [`tswift_core::base64`]).
///
/// ## `real` and non-finite / signed-zero values
///
/// A finite `real` other than negative zero encodes as a bare JSON number.
/// But JSON has no literal for `NaN`/`±inf`, and a bare `-0` re-parses as the
/// integer `0` (losing the sign), so those four cases encode as a **tagged
/// string** payload instead: `{"real":"nan"}`, `{"real":"inf"}`,
/// `{"real":"-inf"}`, `{"real":"-0"}`. Decoding accepts either a JSON number
/// or one of those sentinel strings, so the codec round-trips every `f64`
/// bit-pattern class losslessly (sign of zero included).
///
/// Note: SQLite itself *stores* a bound `NaN` as `NULL` (it has no NaN
/// storage), so a `Real(NaN)` bound as a parameter and read back becomes
/// `Null` — that is SQLite's storage semantic, distinct from (and downstream
/// of) this wire codec, which stays lossless on its own terms.
#[derive(Debug, Clone, PartialEq)]
pub enum DbValue {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl DbValue {
    /// Encode to the tagged JSON document.
    pub fn to_json(&self) -> Json {
        let (tag, payload) = match self {
            DbValue::Null => ("null", Json::Null),
            DbValue::Int(i) => ("int", Json::Int(*i)),
            DbValue::Real(d) => ("real", real_payload(*d)),
            DbValue::Text(s) => ("text", Json::Str(s.clone())),
            DbValue::Blob(bytes) => ("blob", Json::Str(tswift_core::base64::encode(bytes))),
        };
        Json::Object(vec![(tag.to_string(), payload)])
    }

    /// Decode from the tagged JSON document (the inverse of [`Self::to_json`]).
    pub fn from_json(node: &Json) -> Result<DbValue, String> {
        let Json::Object(entries) = node else {
            return Err("db value must be a single-key tagged object".to_string());
        };
        let [(tag, payload)] = entries.as_slice() else {
            return Err("db value must have exactly one tag key".to_string());
        };
        match tag.as_str() {
            "null" => Ok(DbValue::Null),
            "int" => match payload {
                Json::Int(i) => Ok(DbValue::Int(*i)),
                _ => Err("db value tagged `int` must carry a JSON number".to_string()),
            },
            "real" => match payload {
                Json::Double(d) => Ok(DbValue::Real(*d)),
                Json::Int(i) => Ok(DbValue::Real(*i as f64)),
                Json::Str(s) => real_from_sentinel(s).map(DbValue::Real),
                _ => Err(
                    "db value tagged `real` must carry a JSON number or sentinel string"
                        .to_string(),
                ),
            },
            "text" => match payload {
                Json::Str(s) => Ok(DbValue::Text(s.clone())),
                _ => Err("db value tagged `text` must carry a JSON string".to_string()),
            },
            "blob" => match payload {
                Json::Str(s) => tswift_core::base64::decode(s)
                    .map(DbValue::Blob)
                    .ok_or_else(|| "db value tagged `blob` has invalid base64".to_string()),
                _ => Err("db value tagged `blob` must carry a base64 JSON string".to_string()),
            },
            other => Err(format!("unknown db value tag `{other}`")),
        }
    }
}

/// Encode an `f64` as a `real` payload: a bare JSON number for a finite,
/// non-negative-zero value; a sentinel string for `NaN`/`±inf`/`-0`.
fn real_payload(d: f64) -> Json {
    if d.is_nan() {
        Json::Str("nan".to_string())
    } else if d.is_infinite() {
        Json::Str(if d > 0.0 { "inf" } else { "-inf" }.to_string())
    } else if d == 0.0 && d.is_sign_negative() {
        Json::Str("-0".to_string())
    } else {
        Json::Double(d)
    }
}

/// Decode a `real` sentinel string back into its `f64` (inverse of the
/// string arm of [`real_payload`]).
fn real_from_sentinel(s: &str) -> Result<f64, String> {
    match s {
        "nan" => Ok(f64::NAN),
        "inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        "-0" | "-0.0" => Ok(-0.0),
        other => Err(format!(
            "db value tagged `real` has unknown sentinel string `{other}`"
        )),
    }
}

/// Encode a list of bind parameters to the JSON-array-of-tagged-values `String`
/// that travels as `tswift.db.execute`/`tswift.db.query`'s `params` argument.
pub fn encode_params(values: &[DbValue]) -> String {
    json::to_string(&Json::Array(values.iter().map(DbValue::to_json).collect()))
}

/// Decode a `params` wire `String` back into a list of [`DbValue`]s.
pub fn decode_params(text: &str) -> Result<Vec<DbValue>, String> {
    let root = json::parse(text).map_err(|e| format!("invalid params JSON: {e}"))?;
    let Json::Array(items) = root else {
        return Err("params must be a JSON array".to_string());
    };
    items.iter().map(DbValue::from_json).collect()
}

/// One result row: column name paired with its tagged value, in column order
/// (a `Vec`, not a map — SQLite result columns can repeat a name, and column
/// order is part of the documented `query` contract).
pub type DbRow = Vec<(String, DbValue)>;

/// Encode `tswift.db.query`'s reply: a JSON array of column-name-keyed
/// objects, one per row.
///
/// SQLite does not guarantee result column names are unique (e.g.
/// `SELECT a, a FROM t`, or a join projecting two `id` columns). Emitting a
/// JSON object with duplicate keys is technically legal but ambiguous — a
/// downstream JSON map keeps only one. So duplicate names within a row are
/// **disambiguated** by suffixing the second and later occurrences with
/// `_1`, `_2`, … (`a`, `a_1`, `a_2`), keeping every column addressable while
/// preserving column order. The first occurrence keeps its bare name. If a
/// suffixed candidate would itself collide with a real column of that name
/// (e.g. `a, a, a_1`), the suffix counter keeps advancing until the key is
/// unused within the row.
pub fn encode_rows(rows: &[DbRow]) -> String {
    let encoded = rows
        .iter()
        .map(|row| Json::Object(disambiguate_columns(row)))
        .collect();
    json::to_string(&Json::Array(encoded))
}

/// Produce `(key, encoded value)` pairs for one row, suffixing repeated
/// column names so every JSON key is unique (see [`encode_rows`]).
fn disambiguate_columns(row: &DbRow) -> Vec<(String, Json)> {
    use std::collections::{HashMap, HashSet};
    // `next` tracks the suffix counter to *try* next for each base name;
    // `used` tracks every key already emitted so a suffixed candidate that
    // collides with a real column (`a, a, a_1`) keeps advancing until free.
    let mut next: HashMap<&str, u32> = HashMap::new();
    let mut used: HashSet<String> = HashSet::with_capacity(row.len());
    let mut out = Vec::with_capacity(row.len());
    for (name, value) in row {
        let key = if used.contains(name.as_str()) {
            let counter = next.entry(name.as_str()).or_insert(0);
            loop {
                *counter += 1;
                let candidate = format!("{name}_{counter}");
                if !used.contains(&candidate) {
                    break candidate;
                }
            }
        } else {
            name.clone()
        };
        used.insert(key.clone());
        out.push((key, value.to_json()));
    }
    out
}

/// Decode `tswift.db.query`'s reply `String` back into rows.
pub fn decode_rows(text: &str) -> Result<Vec<DbRow>, String> {
    let root = json::parse(text).map_err(|e| format!("invalid rows JSON: {e}"))?;
    let Json::Array(rows) = root else {
        return Err("rows must be a JSON array".to_string());
    };
    rows.iter()
        .map(|row| {
            let Json::Object(entries) = row else {
                return Err("each row must be a JSON object".to_string());
            };
            entries
                .iter()
                .map(|(name, value)| Ok((name.clone(), DbValue::from_json(value)?)))
                .collect()
        })
        .collect()
}

/// The outcome of `tswift.db.execute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecResult {
    pub rows_affected: i64,
    pub last_insert_rowid: i64,
}

impl ExecResult {
    pub fn to_json(self) -> Json {
        Json::Object(vec![
            ("rowsAffected".to_string(), Json::Int(self.rows_affected)),
            (
                "lastInsertRowid".to_string(),
                Json::Int(self.last_insert_rowid),
            ),
        ])
    }

    pub fn encode(self) -> String {
        json::to_string(&self.to_json())
    }

    pub fn decode(text: &str) -> Result<ExecResult, String> {
        let root = json::parse(text).map_err(|e| format!("invalid execute-result JSON: {e}"))?;
        let rows_affected = match root.get("rowsAffected") {
            Some(Json::Int(i)) => *i,
            _ => return Err("execute result missing integer `rowsAffected`".to_string()),
        };
        let last_insert_rowid = match root.get("lastInsertRowid") {
            Some(Json::Int(i)) => *i,
            _ => return Err("execute result missing integer `lastInsertRowid`".to_string()),
        };
        Ok(ExecResult {
            rows_affected,
            last_insert_rowid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_value_round_trips_every_tag() {
        let values = vec![
            DbValue::Null,
            DbValue::Int(-42),
            DbValue::Real(3.5),
            DbValue::Real(5.0), // must not collapse to an Int on the wire.
            DbValue::Text("hi".to_string()),
            DbValue::Blob(vec![0, 1, 2, 255]),
        ];
        for v in values {
            let json = v.to_json();
            let back = DbValue::from_json(&json).unwrap();
            assert_eq!(v, back);
        }
    }

    #[test]
    fn params_round_trip() {
        let values = vec![
            DbValue::Int(1),
            DbValue::Text("a".to_string()),
            DbValue::Null,
        ];
        let text = encode_params(&values);
        let back = decode_params(&text).unwrap();
        assert_eq!(values, back);
    }

    #[test]
    fn rows_round_trip() {
        let rows = vec![
            vec![
                ("id".to_string(), DbValue::Int(1)),
                ("name".to_string(), DbValue::Text("a".to_string())),
            ],
            vec![
                ("id".to_string(), DbValue::Int(2)),
                ("name".to_string(), DbValue::Null),
            ],
        ];
        let text = encode_rows(&rows);
        let back = decode_rows(&text).unwrap();
        assert_eq!(rows, back);
    }

    #[test]
    fn exec_result_round_trips() {
        let result = ExecResult {
            rows_affected: 3,
            last_insert_rowid: 7,
        };
        let text = result.encode();
        assert_eq!(ExecResult::decode(&text).unwrap(), result);
    }

    #[test]
    fn non_finite_and_signed_zero_reals_round_trip_losslessly() {
        // NaN needs bit-comparison (NaN != NaN); check the class + sign.
        let nan = DbValue::from_json(&DbValue::Real(f64::NAN).to_json()).unwrap();
        assert!(matches!(nan, DbValue::Real(d) if d.is_nan()));

        for (value, want) in [
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
            (-0.0f64, -0.0f64),
        ] {
            let back = DbValue::from_json(&DbValue::Real(value).to_json()).unwrap();
            let DbValue::Real(d) = back else {
                panic!("expected Real, got {back:?}");
            };
            assert_eq!(d, want);
            // -0.0 must preserve its sign bit (equality alone can't tell).
            assert_eq!(d.is_sign_negative(), want.is_sign_negative());
        }
    }

    #[test]
    fn non_finite_reals_encode_as_valid_json() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0] {
            let text = json::to_string(&DbValue::Real(value).to_json());
            // Must be re-parseable JSON (bare `nan`/`inf` would not be).
            assert!(json::parse(&text).is_ok(), "{text}");
        }
    }

    #[test]
    fn duplicate_column_names_are_disambiguated() {
        let rows = vec![vec![
            ("a".to_string(), DbValue::Int(1)),
            ("a".to_string(), DbValue::Int(2)),
            ("a".to_string(), DbValue::Int(3)),
            ("b".to_string(), DbValue::Int(4)),
        ]];
        let text = encode_rows(&rows);
        let back = decode_rows(&text).unwrap();
        let keys: Vec<&str> = back[0].iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a", "a_1", "a_2", "b"]);
        // Every value preserved, in column order.
        let vals: Vec<&DbValue> = back[0].iter().map(|(_, v)| v).collect();
        assert_eq!(
            vals,
            [
                &DbValue::Int(1),
                &DbValue::Int(2),
                &DbValue::Int(3),
                &DbValue::Int(4)
            ]
        );
    }

    #[test]
    fn suffixed_name_collision_keeps_advancing() {
        // `a, a, a_1`: the second `a` becomes `a_1`; the *real* third column
        // `a_1` would then collide, so its suffix counter keeps advancing to
        // `a_1_1`. Old (buggy) code emitted two `a_1` keys.
        let rows = vec![vec![
            ("a".to_string(), DbValue::Int(1)),
            ("a".to_string(), DbValue::Int(2)),
            ("a_1".to_string(), DbValue::Int(3)),
        ]];
        let text = encode_rows(&rows);
        let back = decode_rows(&text).unwrap();
        let keys: Vec<&str> = back[0].iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["a", "a_1", "a_1_1"]);
        // No duplicate keys emitted.
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
    }

    #[test]
    fn from_json_rejects_malformed_tags() {
        assert!(DbValue::from_json(&Json::Null).is_err());
        let empty = Json::Object(vec![]);
        assert!(DbValue::from_json(&empty).is_err());
        let unknown = Json::Object(vec![("weird".to_string(), Json::Null)]);
        assert!(DbValue::from_json(&unknown).is_err());
        let bad_int = Json::Object(vec![("int".to_string(), Json::Str("nope".to_string()))]);
        assert!(DbValue::from_json(&bad_int).is_err());
    }

    #[test]
    fn host_fn_signatures_are_well_formed() {
        for sig in HOST_FN_SIGNATURES {
            tswift_core::HostSignature::from_json(sig).unwrap();
        }
    }
}
