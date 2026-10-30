//! `uniffi-bindgen`: reads the core's host library and writes the Swift or
//! Kotlin bindings (core/BUILD.bazel runs it). The uniffi crate's own
//! command does the same through `cargo metadata`, which a Bazel sandbox
//! does not have; this one takes the library path and nothing else.
//!
//! `uniffi-bindgen --library <cdylib> --language <kotlin|swift> --out-dir <dir>`

use std::process::ExitCode;

use camino::Utf8PathBuf;
use uniffi_bindgen::bindings::{GenerateOptions, TargetLanguage, generate};

fn main() -> ExitCode {
    let mut library = None;
    let mut language = None;
    let mut out_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library = args.next(),
            "--language" => language = args.next(),
            "--out-dir" => out_dir = args.next(),
            "generate" | "--no-format" => {}
            other => {
                eprintln!("uniffi-bindgen: unknown argument {other:?}");
                return ExitCode::from(2);
            }
        }
    }
    let (Some(library), Some(language), Some(out_dir)) = (library, language, out_dir) else {
        eprintln!("usage: uniffi-bindgen --library <cdylib> --language <kotlin|swift> --out-dir <dir>");
        return ExitCode::from(2);
    };
    let language = match language.as_str() {
        "kotlin" => TargetLanguage::Kotlin,
        "swift" => TargetLanguage::Swift,
        other => {
            eprintln!("uniffi-bindgen: language {other:?} is not kotlin or swift");
            return ExitCode::from(2);
        }
    };
    let options = GenerateOptions {
        languages: vec![language],
        source: Utf8PathBuf::from(library),
        out_dir: Utf8PathBuf::from(out_dir),
        config_override: None,
        format: false,
        crate_filter: None,
        metadata_no_deps: false,
    };
    match generate(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("uniffi-bindgen: {e:#}");
            ExitCode::FAILURE
        }
    }
}
