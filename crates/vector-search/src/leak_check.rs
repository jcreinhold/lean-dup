use crate::{Error, Result};

pub(crate) fn check(json: &str) -> Result<()> {
    for pattern in forbidden_patterns() {
        if json.contains(pattern) {
            return Err(Error::Leak {
                message: format!("artifact contains forbidden pattern `{pattern}`"),
            });
        }
    }
    Ok(())
}

fn forbidden_patterns() -> [&'static str; 15] {
    [
        "/Users/",
        concat!("qu", "ery", ":"),
        concat!("pass", "age", ":"),
        "LanceDB",
        "lancedb",
        "\"table\"",
        "worker row",
        "worker_rows",
        "retrieval_key",
        "sqlite",
        "posting",
        "tokenizer",
        "onnx",
        "FastEmbed",
        "fastembed",
    ]
}
