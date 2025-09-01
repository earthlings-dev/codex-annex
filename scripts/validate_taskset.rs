// tiny schema validator for TaskSet JSON files
use std::{env, fs};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).expect("usage: validate_taskset <file.json>");
    let schema_text = include_str!("../schemas/taskset.schema.json");
    let schema: serde_json::Value = serde_json::from_str(schema_text)?;
    let data_text = fs::read_to_string(&path)?;
    let data: serde_json::Value = serde_json::from_str(&data_text)?;
    // Use jsonschema crate for validation
    let compiled = jsonschema::JSONSchema::compile(&schema)?;
    if let Err(errors) = compiled.validate(&data) {
        eprintln!("Invalid: {}", path);
        for e in errors { eprintln!("- {}", e); }
        std::process::exit(1);
    }
    println!("OK: {}", path);
    Ok(())
}
