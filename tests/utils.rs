#![allow(unused)]
use std::ffi::{OsStr, OsString};
use std::io::prelude::*;
use std::{env, fs, path::Path, path::PathBuf, process};

/// Create temporary working directory with Cargo.toml manifest
pub fn clone_out_test(source: &str) -> (tempfile::TempDir, String) {
    let tmpdir = tempfile::tempdir().expect("failed to construct temporary directory");
    fs::copy(source, tmpdir.path().join("Cargo.toml"))
        .unwrap_or_else(|err| panic!("could not copy test manifest: {}", err));
    let path = tmpdir
        .path()
        .join("Cargo.toml")
        .to_str()
        .unwrap()
        .to_string()
        .clone();

    (tmpdir, path)
}

/// Add directory
pub fn setup_alt_registry_config(path: &std::path::Path) {
    fs::create_dir(path.join(".cargo")).expect("failed to create .cargo directory");
    fs::copy(
        "tests/fixtures/alt-registry-cargo-config",
        path.join(".cargo").join("config"),
    )
    .unwrap_or_else(|err| panic!("could not copy test cargo config: {}", err));
}

/// Execute local cargo command, includes `--manifest-path`, expect command failed
pub fn execute_bad_command<S>(command: &[S], manifest: &str)
where
    S: AsRef<OsStr>,
{
    let subcommand_name = &command[0].as_ref();

    let call = process::Command::new(&get_command_path(subcommand_name))
        .args(command)
        .arg(format!("--manifest-path={}", manifest))
        .env("CARGO_IS_TEST", "1")
        .output()
        .unwrap();

    if call.status.success() {
        println!("Status code: {:?}", call.status);
        println!("STDOUT: {}", String::from_utf8_lossy(&call.stdout));
        println!("STDERR: {}", String::from_utf8_lossy(&call.stderr));
        panic!(
            "cargo-{} success to execute",
            subcommand_name.to_string_lossy()
        )
    }
}

/// Execute local cargo command, includes `--manifest-path`
pub fn execute_command<S>(command: &[S], manifest: &str)
where
    S: AsRef<OsStr>,
{
    let subcommand_name = &command[0].as_ref();

    let call = process::Command::new(&get_command_path(subcommand_name))
        .args(command)
        .arg(format!("--manifest-path={}", manifest))
        .env("CARGO_IS_TEST", "1")
        .output()
        .expect("call to test build failed");

    if !call.status.success() {
        println!("Status code: {:?}", call.status);
        println!("STDOUT: {}", String::from_utf8_lossy(&call.stdout));
        println!("STDERR: {}", String::from_utf8_lossy(&call.stderr));
        panic!(
            "cargo-{} failed to execute",
            subcommand_name.to_string_lossy()
        )
    }
}

/// Execute local cargo command in a given directory
pub fn execute_command_in_dir<S>(command: &[S], dir: &Path)
where
    S: AsRef<OsStr>,
{
    let subcommand_name = &command[0].as_ref();

    let call = process::Command::new(&get_command_path(subcommand_name))
        .args(command)
        .env("CARGO_IS_TEST", "1")
        .current_dir(dir)
        .output()
        .expect("call to test build failed");

    if !call.status.success() {
        println!("Status code: {:?}", call.status);
        println!("STDOUT: {}", String::from_utf8_lossy(&call.stdout));
        println!("STDERR: {}", String::from_utf8_lossy(&call.stderr));
        panic!(
            "cargo-{} failed to execute",
            subcommand_name.to_string_lossy()
        )
    }
}

/// Parse a manifest file as TOML
pub fn get_toml(manifest_path: &str) -> toml_edit::Document {
    let mut f = fs::File::open(manifest_path).unwrap();
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    s.parse().expect("toml parse error")
}

pub fn get_command_path(s: impl AsRef<OsStr>) -> String {
    let target_dir: PathBuf = match env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => dir.into(),
        None => env::current_dir()
            .expect("Failed to get current dir")
            .join("target"),
    };

    let mut binary_name = OsString::from("cargo-");
    binary_name.push(s.as_ref());

    target_dir
        .join("debug")
        .join(binary_name)
        .to_str()
        .unwrap()
        .to_string()
}
