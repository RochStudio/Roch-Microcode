//! Minimal harness shared by the fixture suites.
//!
//! The suites behave like the original .NET console test programs: each check
//! prints `PASS <name>`; the first failure prints `FAIL <name>: <reason>` and
//! exits with status 1. They never flash hardware or write to USB drives.

use std::path::{Path, PathBuf};

use roch_core::{Error, Result};

pub type CheckResult = std::result::Result<(), String>;

#[derive(Default)]
pub struct Suite {
    pub passed: usize,
}

impl Suite {
    pub fn check(&mut self, name: impl AsRef<str>, test: impl FnOnce() -> CheckResult) {
        let name = name.as_ref();
        match test() {
            Ok(()) => {
                println!("PASS {name}");
                self.passed += 1;
            }
            Err(reason) => fail(&format!("FAIL {name}: {reason}")),
        }
    }
}

pub fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// `Assert(condition)`.
pub fn ensure(ok: bool) -> CheckResult {
    if ok {
        Ok(())
    } else {
        Err("Assertion failed".into())
    }
}

/// Convert an engine error into a check failure.
pub fn ok<T>(r: Result<T>) -> std::result::Result<T, String> {
    r.map_err(|e| format!("unexpected error: {e}"))
}

/// Unsafe input must be rejected with an invalid-data error (`InvalidDataException`).
pub fn reject_invalid<T>(r: Result<T>) -> CheckResult {
    match r {
        Err(Error::InvalidData(_)) => Ok(()),
        Err(e) => Err(format!("rejected with the wrong error kind: {e:?}")),
        Ok(_) => Err("Unsupported modification accepted".into()),
    }
}

/// Unsafe input must be rejected with an invalid-data or I/O error.
pub fn reject_any<T>(r: Result<T>) -> CheckResult {
    match r {
        Err(Error::InvalidData(_)) | Err(Error::Io(_)) => Ok(()),
        Err(e) => Err(format!("rejected with the wrong error kind: {e:?}")),
        Ok(_) => Err("Unsafe input was accepted".into()),
    }
}

/// `Directory.EnumerateFiles(root, file, AllDirectories).Single()`.
pub fn find_single(root: &Path, file: &str) -> PathBuf {
    fn walk(dir: &Path, file: &str, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, file, out);
            } else if path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(file)) {
                out.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(root, file, &mut found);
    if found.len() != 1 {
        fail(&format!("Expected exactly one {file} under {}, found {}.", root.display(), found.len()));
    }
    found.remove(0)
}

pub fn temp_root(prefix: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("{prefix}-{}", roch_core::format::guid_n()));
    if let Err(e) = std::fs::create_dir_all(&root) {
        fail(&format!("Could not create {}: {e}", root.display()));
    }
    root
}
