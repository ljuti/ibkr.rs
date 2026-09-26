//! Conformance vectors: the gateway contract as data, not prose.
//!
//! `conformance/vectors/*.json` pins what a conforming consumer must do with a
//! Flex report payload — decode it, partition its levels of detail, store it,
//! and answer the same questions from the result. The file is the
//! specification; this test is one implementation of it (the gateway's own
//! tests, or a port in another language, can run the same cases).
//!
//! Run with `cargo test --test conformance`. See `conformance/README.md` for
//! the field-by-field format.

use std::fs;
use std::path::{Path, PathBuf};

use ibkr::store::{Store, SyncStats};
use ibkr::types::FlexReportResponse;
use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("conformance")
        .join("vectors")
}

/// A store in its own temporary directory, so nothing is left behind.
fn store_path(case: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("ibkr-conformance-{case}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create the case directory");
    directory.join("trades.db")
}

fn cleanup(path: &Path) {
    if let Some(directory) = path.parent() {
        let _ = fs::remove_dir_all(directory);
    }
}

#[test]
fn flex_vectors_hold() {
    let mut cases = 0;
    let mut failures = Vec::new();

    let entries = fs::read_dir(vectors_dir()).expect("conformance/vectors exists");
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read the vector file");
        let file: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{} is not valid JSON: {error}", path.display()));
        for case in file["cases"].as_array().expect("cases is an array") {
            cases += 1;
            let name = case["name"].as_str().unwrap_or("<unnamed>");
            if let Err(problem) = check(case) {
                failures.push(format!("{name}: {problem}"));
            }
        }
    }

    assert!(
        cases > 0,
        "no conformance cases in {}",
        vectors_dir().display()
    );
    assert!(
        failures.is_empty(),
        "{} of {cases} conformance cases failed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Apply one case and check every expectation it states.
///
/// The case directory is removed whichever way the case ends: a failing vector
/// is exactly when the next run needs a clean slate.
fn check(case: &Value) -> Result<(), String> {
    let path = store_path(case["name"].as_str().unwrap_or("case"));
    let outcome = check_case(case, &path);
    cleanup(&path);
    outcome
}

fn check_case(case: &Value, path: &Path) -> Result<(), String> {
    let mut store = Store::open(path).map_err(|error| format!("cannot open a store: {error}"))?;

    // The payload must decode: that is the first expectation of every case.
    let payload: FlexReportResponse = serde_json::from_value(case["raw"].clone())
        .map_err(|error| format!("the raw payload does not decode as a Flex report: {error}"))?;
    let report = case["report"].as_str().unwrap_or("vector");
    let stats = store
        .upsert_report(report, None, &payload)
        .map_err(|error| format!("storing the payload failed: {error}"))?;

    let expect = &case["expect"];
    if let Some(payload_expectations) = expect["payload"].as_array() {
        let decoded = serde_json::to_value(&payload).map_err(|error| error.to_string())?;
        for assertion in payload_expectations {
            let path = assertion["path"].as_str().unwrap_or_default();
            let expected = &assertion["equals"];
            let actual = at_path(&decoded, path);
            if actual != expected {
                return Err(format!("payload {path}: expected {expected}, got {actual}"));
            }
        }
    }

    if let Some(ingest) = expect["ingest"].as_object() {
        check_counts(&stats, ingest)?;
    }

    // Overlapping windows: the same rows arriving again must not multiply.
    for extra in expect["extraSyncs"].as_array().into_iter().flatten() {
        let extra_payload: FlexReportResponse = serde_json::from_value(extra["raw"].clone())
            .map_err(|error| format!("an extraSync payload does not decode: {error}"))?;
        let extra_report = extra["report"].as_str().unwrap_or("vector-extra");
        store
            .upsert_report(extra_report, None, &extra_payload)
            .map_err(|error| format!("the extraSync for {extra_report} failed: {error}"))?;
    }

    for query in expect["queries"].as_array().into_iter().flatten() {
        let sql = query["sql"].as_str().unwrap_or_default();
        let result = store
            .query(sql)
            .map_err(|error| format!("`{sql}` failed: {error}"))?;
        let actual: Vec<Value> = result
            .rows
            .iter()
            .map(|row| Value::Array(row.iter().map(sql_value).collect()))
            .collect();
        let expected = query["rows"].clone();
        if !rows_match(&actual, &expected) {
            return Err(format!(
                "`{sql}`:\n      expected {expected}\n      got      {}",
                Value::Array(actual)
            ));
        }
    }

    Ok(())
}

fn check_counts(
    stats: &SyncStats,
    expected: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    let actual = json!({
        "executions": stats.executions,
        "lots": stats.lots,
        "cashTransactions": stats.cash_transactions,
        "skipped": stats.skipped,
        "cashDated": stats.cash_dated,
    });
    for (field, want) in expected {
        let got = &actual[field];
        if got != want {
            return Err(format!("ingest {field}: expected {want}, got {got}"));
        }
    }
    Ok(())
}

/// Walk `a.b.0.c` through the decoded payload.
fn at_path<'a>(value: &'a Value, path: &str) -> &'a Value {
    let mut current = value;
    for step in path.split('.') {
        current = match step.parse::<usize>() {
            Ok(index) => current.get(index).unwrap_or(&Value::Null),
            Err(_) => current.get(step).unwrap_or(&Value::Null),
        };
    }
    current
}

/// `SQLite` cell → JSON, keeping the types the queries compare against.
fn sql_value(cell: &SqlValue) -> Value {
    match cell {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(number) => json!(number),
        SqlValue::Real(number) => Value::Number(
            serde_json::Number::from_f64(*number).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
        SqlValue::Text(text) => json!(text),
        SqlValue::Blob(bytes) => json!(format!("<{} bytes>", bytes.len())),
    }
}

/// Compare rows numerically, so `2` and `2.0` are the same answer.
fn rows_match(actual: &[Value], expected: &Value) -> bool {
    let Some(expected) = expected.as_array() else {
        return false;
    };
    if actual.len() != expected.len() {
        return false;
    }
    actual
        .iter()
        .zip(expected)
        .all(|(row, want)| cells_match(row, want))
}

fn cells_match(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Array(a), Value::Array(e)) => {
            a.len() == e.len() && a.iter().zip(e).all(|(a, e)| cells_match(a, e))
        }
        (Value::Number(a), Value::Number(e)) => match (a.as_f64(), e.as_f64()) {
            (Some(a), Some(e)) => (a - e).abs() < 1e-9,
            _ => a == e,
        },
        _ => actual == expected,
    }
}
