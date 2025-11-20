use anyhow::Result;
use conjure_cp::context::Context;
use conjure_cp::parse::tree_sitter::parse_essence_file_native;
use conjure_cp_cli::utils::testing::{read_model_json, save_model_json};
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// This test file is specifially for testing the native parser

fn copy_generated_to_expected(
    path: &str,
    test_name: &str,
    stage: &str,
    extension: &str,
) -> Result<(), std::io::Error> {
    std::fs::copy(
        format!("{path}/{test_name}.generated-{stage}.{extension}"),
        format!("{path}/{test_name}.expected-{stage}.{extension}"),
    )?;
    Ok(())
}

#[derive(Clone, Debug, clap::Args)]
pub struct Args {
    /// The Essence test directory
    #[arg(default_value = "tests-integration/tests")]
    pub test_directory: PathBuf,

    /// Accept current output as expected (update .expected files)
    #[arg(long)]
    pub accept: bool,
}

#[derive(Deserialize)]
struct TestConfig {
    enable_native_parser: Option<bool>,
}

pub fn run_parse_test_command(parse_test_args: Args) -> Result<()> {
    let test_path = &parse_test_args.test_directory;
    let accept =
        parse_test_args.accept || env::var("ACCEPT").unwrap_or("false".to_string()) == "true";

    // Check existence of test directory
    if !test_path.exists() {
        anyhow::bail!("Test directory does not exist: {}", test_path.display());
    }

    // Find essence files recursively
    let essence_files = find_essence_files_recursive(test_path)?;

    if essence_files.is_empty() {
        anyhow::bail!(
            "No .essence or .eprime files found in {}",
            test_path.display()
        );
    }

    println!("Found {} essence files to test", essence_files.len());

    let mut passed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    // Iterate through all test files
    for essence_file in essence_files {
        let context: Arc<RwLock<Context<'static>>> = Default::default();
        let path = &essence_file.to_string_lossy();
        let test_dir = &essence_file.parent().unwrap().to_string_lossy();
        let essence_base = &essence_file.file_stem().unwrap().to_string_lossy();

        // Check if config.toml in test directory allows native parser
        let use_native_parser: bool =
            if let Ok(config_contents) = fs::read_to_string(format!("{}/config.toml", test_dir)) {
                match toml::from_str::<TestConfig>(&config_contents) {
                    Ok(cfg) => cfg.enable_native_parser.unwrap_or(true),
                    Err(e) => {
                        println!(
                            "{}: Failed to parse config.toml: {}",
                            path.to_string(),
                            e
                        );
                        true
                    }
                }
            } else {
                true
            };

        if !use_native_parser {
            println!(
                "{}: Skipped because native parser disabled in config.toml",
                path.to_string()
            );
            continue;
        }

        // Parse the file
        match std::panic::catch_unwind(|| parse_essence_file_native(path, context.clone())) {
            Ok(Ok(model)) => {
                save_model_json(&model, test_dir, essence_base, "parse")?;
                model
            }
            Ok(Err(e)) => {
                println!("{}: Parse error: {}", path.to_string(), e);
                failed.push(path.to_string());
                continue;
            }
            Err(payload) => {
                let panic_msg = if let Some(s) = (&payload).downcast_ref::<&'static str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Parser panicked: non-string payload".to_string()
                };
                println!("{}: Parser panicked: {}", path.to_string(), panic_msg);
                failed.push(path.to_string());
                continue;
            }
        };

        // Find expected json parse in test directory
        match read_model_json(&context, test_dir, essence_base, "expected", "parse") {
            Ok(_) => match compare_json_file(test_dir, essence_base, accept) {
                Ok(equal) => {
                    if equal {
                        println!("{}: Passed", path.to_string());
                        passed.push(path.to_string());
                        continue;
                    }
                    if accept {
                        match copy_generated_to_expected(
                            test_dir,
                            essence_base,
                            "parse",
                            "serialised.json",
                        ) {
                            Ok(_) => passed.push(path.to_string()),
                            Err(e) => {
                                println!(
                                    "Failed to save expected model for {}: {}",
                                    essence_base, e
                                );
                                failed.push(path.to_string());
                            }
                        }
                    } else {
                        println!(
                            "{}: Parsed model doesn't match expected",
                            path.to_string()
                        );
                        failed.push(path.to_string());
                    }
                }
                Err(e) => {
                    println!(
                        "{}: Error comparing expected and generated results: {}",
                        path.to_string(),
                        e
                    );
                    failed.push(path.to_string());
                }
            },
            Err(e) => {
                if accept {
                    match copy_generated_to_expected(
                        test_dir,
                        essence_base,
                        "parse",
                        "serialised.json",
                    ) {
                        Ok(_) => passed.push(path.to_string()),
                        Err(e) => {
                            println!("Failed to save expected model for {}: {}", essence_base, e);
                            failed.push(path.to_string());
                        }
                    }
                } else {
                    println!(
                        "{}: Expected model could not be found: {}",
                        path.to_string(),
                        e
                    );
                    failed.push(path.to_string());
                    continue;
                }
            }
        }
    }

    // Summary of results
    println!("\nParser test results:");

    for f in &failed {
        println!("  FAILED: {}", f);
    }

    println!("\nParser tests: {} passed, {} failed", passed.len(), failed.len());

    Ok(())
}

fn find_essence_files_recursive(dir: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut essence_files = Vec::new();
    find_essence_files_recursive_helper(dir, &mut essence_files)?;
    Ok(essence_files)
}

fn find_essence_files_recursive_helper(
    dir: &PathBuf,
    essence_files: &mut Vec<PathBuf>,
) -> Result<()> {
    use std::fs;

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "essence" || ext == "eprime" { essence_files.push(path); }
            }
        } else if path.is_dir() {
            find_essence_files_recursive_helper(&path, essence_files)?;
        }
    }

    Ok(())
}

fn compare_json_file(test_dir: &str, base: &str, accept: bool) -> Result<bool> {
    let gen_path = format!("{}/{}.generated-parse.serialised.json", test_dir, base);
    let exp_path = format!("{}/{}.expected-parse.serialised.json", test_dir, base);

    let gen_raw = match fs::read_to_string(&gen_path) {
        Ok(s) => s,
        Err(e) => {
            println!("Error reading {}: {}", gen_path, e);
            return Err(anyhow::anyhow!("Error reading {}: {}", gen_path, e));
        }
    };

    let exp_raw = match fs::read_to_string(&exp_path) {
        Ok(s) => s,
        Err(e) => {
            println!("Error reading {}: {}", exp_path, e);
            return Err(anyhow::anyhow!("Error reading {}: {}", exp_path, e));
        }
    };

    let gen_val: serde_json::Value = serde_json::from_str(&gen_raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON {}: {}", gen_path, e))?;
    let exp_val: serde_json::Value = serde_json::from_str(&exp_raw)
        .map_err(|e| anyhow::anyhow!("Failed to parse JSON {}: {}", exp_path, e))?;

    let gen_string = serde_json::to_string_pretty(&gen_val)?;
    let exp_string = serde_json::to_string_pretty(&exp_val)?;

    if gen_string == exp_string {
        return Ok(true);
    }

    let gen_lines: Vec<&str> = gen_string.lines().collect();
    let exp_lines: Vec<&str> = exp_string.lines().collect();
    let max = std::cmp::min(gen_lines.len(), exp_lines.len());

    // Check for extra lines in file
    if gen_lines.len() != exp_lines.len() {
        println!(
            "Number of lines different from expected: expected {} lines, generated {} lines",
            exp_lines.len(),
            gen_lines.len()
        );
        return Ok(false)
    }

    let mut diffs: Vec<(usize, &str, &str)> = Vec::new();

    for i in 0..max {
        if gen_lines[i] != exp_lines[i] {
            diffs.push((i, exp_lines[i], gen_lines[i]));
        }
    }

    // Check for line differences and display them
    if !diffs.is_empty() || !accept {
        println!("{}: Parsed result does not match expected (expected | generated)", gen_path);
        for (i, gs, es) in diffs {
            println!("{:6} |    {} | {}", i + 1, gs.trim(), es.trim());
        }
        return Ok(false)
    } 

    Ok(true)
}
