use serde_json::{json, Value};
use std::{fs, io::{self, BufRead}, path::Path};

fn run(input: &str) -> Result<Value, &'static str> {
    let request: Value = serde_json::from_str(input).map_err(|_| "invalid_json")?;
    let request_id = request.get("request_id")
        .and_then(Value::as_str).ok_or("missing_request_id")?;
    if request.get("operation").and_then(Value::as_str) != Some("collect") {
        return Err("unsupported_operation");
    }
    let topic = request.get("arguments")
        .and_then(|v| v.get("topic"))
        .and_then(Value::as_str).ok_or("invalid_arguments")?;
    if topic.is_empty() || topic.len() > 80 {
        return Err("invalid_arguments");
    }
    let records = json!({
        "source_id": "fixture",
        "records": [{"id":"rust-fixture-001","topic":topic,"text":"Deterministic offline Rust result"}]
    });
    let data = serde_json::to_vec_pretty(&records).map_err(|_| "serialization_failed")?;
    let dir = Path::new("/work/out");
    fs::write(dir.join("records.json"), &data).map_err(|_| "artifact_write_failed")?;
    Ok(json!({
        "schema_version": 1,
        "request_id": request_id,
        "ok": true,
        "result": {"topic":topic,"count":1},
        "artifacts": [{"path":"records.json","media_type":"application/json","size_bytes":data.len()}]
    }))
}

fn main() {
    let stdin = io::stdin();
    let Some(Ok(line)) = stdin.lock().lines().next() else {
        std::process::exit(1);
    };
    match run(&line) {
        Ok(response) => println!("{}", response),
        Err(code) => {
            eprintln!("rust_fixture_error={code}");
            std::process::exit(2);
        }
    }
}
