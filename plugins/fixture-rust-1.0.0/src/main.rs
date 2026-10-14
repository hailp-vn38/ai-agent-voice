use serde_json::{json, Value};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

fn run(input: &str) -> Result<Value, &'static str> {
    if input.len() > 65535 || !input.ends_with('\n') || input[..input.len() - 1].contains('\n') {
        return Err("invalid_request");
    }
    let request: Value = serde_json::from_str(input).map_err(|_| "invalid_json")?;
    let dir = Path::new(
        request
            .get("artifact_dir")
            .and_then(Value::as_str)
            .ok_or("invalid_request")?,
    );
    if request.get("schema_version").and_then(Value::as_u64) != Some(1)
        || !dir.is_absolute()
        || dir
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err("invalid_request");
    }
    let request_id = request
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("missing_request_id")?;
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("invalid_request");
    }
    if request.get("operation").and_then(Value::as_str) != Some("collect") {
        return Err("unsupported_operation");
    }
    let topic = request
        .get("arguments")
        .and_then(|v| v.get("topic"))
        .and_then(Value::as_str)
        .ok_or("invalid_arguments")?;
    if topic.is_empty() || topic.chars().count() > 80 {
        return Err("invalid_arguments");
    }
    let records = json!({
        "source_id": "fixture",
        "records": [{"id":"rust-fixture-001","topic":topic,"text":"Deterministic offline Rust result"}]
    });
    let data = serde_json::to_vec_pretty(&records).map_err(|_| "serialization_failed")?;
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
    let mut input = String::new();
    if io::stdin()
        .lock()
        .take(65536)
        .read_to_string(&mut input)
        .is_err()
    {
        std::process::exit(1);
    }
    let response = match run(&input) {
        Ok(response) => response,
        Err(code) => {
            let request: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
            json!({
                "schema_version":1,
                "request_id":request.get("request_id").and_then(Value::as_str).unwrap_or("unknown"),
                "ok":false,"error":{"code":code,"message":"Fixture request failed"},"artifacts":[]
            })
        }
    };
    println!("{}", response);
}

#[cfg(test)]
mod tests;
