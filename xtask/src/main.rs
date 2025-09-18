use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::{fs, path::PathBuf};

#[derive(Parser)]
#[command(name = "xtask", about = "Annex workspace tasks")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Validate a TaskSet JSON file against schemas/taskset.schema.json
    ValidateTaskset { file: PathBuf },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::ValidateTaskset { file } => validate_taskset(&file),
    }
}

fn validate_taskset(path: &PathBuf) -> Result<()> {
    let schema_text = include_str!("../../schemas/taskset.schema.json");
    let schema: serde_json::Value = serde_json::from_str(schema_text)?;
    let compiled = jsonschema::validator_for(&schema)?;
    let data_text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let data: serde_json::Value = serde_json::from_str(&data_text).with_context(|| "parse json")?;
    let errors: Vec<_> = compiled.iter_errors(&data).collect();
    if !errors.is_empty() {
        eprintln!("Invalid: {}", path.display());
        for e in errors {
            eprintln!("- {}", e);
        }
        std::process::exit(1);
    }
    println!("OK: {}", path.display());
    Ok(())
}
