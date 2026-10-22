//! Source discovery errors through the command-line entry point.

use std::path::Path;
use std::process::Command;

fn cli(command: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tov"));
    cmd.current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).arg(command);
    cmd
}

#[test]
fn directories_without_sources_report_the_extension() {
    for dir in ["tests/cli/ts_only", "tests/cli/ts_only/src", "tests/cli/empty"] {
        for command in ["check", "build", "run", "test"] {
            let out = cli(command).arg(dir).output().expect("run compiler");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "{command} {dir} unexpectedly succeeded");
            assert!(stderr.contains("no `.tov` source files found"), "{command} {dir}: {stderr}");
            assert!(stderr.contains("pass a source file explicitly"), "{command} {dir}: {stderr}");
            assert!(out.stdout.is_empty(), "unexpected source diagnostic: {}", String::from_utf8_lossy(&out.stdout));
        }
    }
}

#[test]
fn explicit_typescript_file_still_checks() {
    let out = cli("check").args(["--json", "tests/cli/ts_only/src/main.ts"]).output().expect("run compiler");
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"files\":1"));
}

#[test]
fn tov_directories_still_check() {
    let out = cli("check").args(["--json", "examples/shapes"]).output().expect("run compiler");
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("\"errors\":0"));
}

#[test]
fn help_after_a_command_prints_help_instead_of_running_it() {
    for command in ["clean", "check", "build", "run", "test", "explain"] {
        let out = cli(command).arg("--help").output().expect("run compiler");
        assert!(out.status.success(), "{command} --help failed");
        assert!(String::from_utf8_lossy(&out.stdout).contains("usage:"), "{command} --help printed no help");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("freed"), "{command} --help ran the command");
    }
}

#[test]
fn test_directories_run_each_typescript_test_file() {
    let out = cli("test").arg("tests/cli/jest_dir").output().expect("run compiler");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "a file that doesn't build fails the run: {stdout}");
    assert!(stdout.contains("ok   adds > one and two"), "{stdout}");
    assert!(stdout.contains("unknown name `missing`"), "{stdout}");
    assert!(stdout.contains("1 passed, 0 failed across 2 files (1 didn't build)"), "{stdout}");
}
