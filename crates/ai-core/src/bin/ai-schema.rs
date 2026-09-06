//! Generates or checks the committed contract schemas.
//!
//! `cargo run -p ai-core --features schema-tool --bin ai-schema -- generate`
//! `cargo run -p ai-core --features schema-tool --bin ai-schema -- generate --check`
//!
//! `--check` writes nothing and fails when the committed files differ, which is
//! what CI runs to catch contract drift.

use std::{path::PathBuf, process::ExitCode};

use ai_core::schema::{GeneratedSchema, generate_all};

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let check = arguments.iter().any(|argument| argument == "--check");
    let generate = arguments.iter().any(|argument| argument == "generate");

    if !generate {
        eprintln!("usage: ai-schema generate [--check]");
        return ExitCode::FAILURE;
    }

    match run(check) {
        Ok(drifted) if drifted.is_empty() => {
            println!(
                "{} schemas {}",
                generate_all().len(),
                if check { "match" } else { "written" }
            );
            ExitCode::SUCCESS
        }
        Ok(drifted) => {
            for name in drifted {
                eprintln!("schema drift: {name}.json differs from the generated schema");
            }
            eprintln!("run without --check to update the committed schemas");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("schema tool failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(check: bool) -> std::io::Result<Vec<&'static str>> {
    let directory = schema_directory();
    std::fs::create_dir_all(&directory)?;

    let mut drifted = Vec::new();
    for GeneratedSchema { name, schema } in generate_all() {
        let path = directory.join(format!("{name}.json"));
        let mut rendered = serde_json::to_string_pretty(&schema)?;
        rendered.push('\n');

        if check {
            match std::fs::read_to_string(&path) {
                Ok(committed) if committed == rendered => {}
                _ => drifted.push(name),
            }
        } else {
            std::fs::write(&path, rendered)?;
        }
    }
    Ok(drifted)
}

fn schema_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../specs/schemas")
        .components()
        .collect()
}
