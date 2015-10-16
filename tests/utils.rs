extern crate tempdir;
extern crate toml;

use std::{fs, process};
use std::io::prelude::*;
use std::ffi::OsStr;

pub fn clone_out_test(source: &str) -> (tempdir::TempDir, String) {
    let tmpdir = tempdir::TempDir::new("cargo-add-test")
        .ok().expect("failed to construct temporary directory");
    fs::copy(source, tmpdir.path().join("Cargo.toml"))
        .unwrap_or_else(|err| panic!("could not copy test manifest: {}", err));
    let path = tmpdir.path().join("Cargo.toml").to_str().unwrap().to_string().clone();

    (tmpdir, path)
}

pub fn execute_command<S>(command: &[S], manifest: &str) where S: AsRef<OsStr> {
    let subcommand_name = &command[0].as_ref().to_str().unwrap();

    let call = process::Command::new(&format!("target/debug/cargo-{}", subcommand_name))
        .args(command)
        .arg(format!("--manifest-path={}", manifest))
        .env("CARGO_IS_TEST", "1")
        .output().unwrap();

    if !call.status.success() {
        println!("Status code: {:?}", call.status);
        println!("STDOUT: {}", String::from_utf8_lossy(&call.stdout));
        println!("STDERR: {}", String::from_utf8_lossy(&call.stderr));
        panic!("cargo-add failed to execute")
    }
}

pub fn get_toml(manifest_path: &str) -> toml::Value {
    let mut f = fs::File::open(manifest_path).unwrap();
    let mut s = String::new();
    f.read_to_string(&mut s).unwrap();
    toml::Value::Table(toml::Parser::new(&s).parse().unwrap())
}

/// 'failure' dep not present
pub fn no_manifest_failures(manifest: &toml::Value) -> bool {
    manifest.lookup("dependencies.failure").is_none() &&
    manifest.lookup("dev-dependencies.failure").is_none() &&
    manifest.lookup("build-dependencies.failure").is_none()
}
