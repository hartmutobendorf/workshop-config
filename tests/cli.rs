use std::{
    fs,
    process::{Command, Output},
};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_workshop-tui"))
        .args(args)
        .env("NO_COLOR", "1")
        .env_remove("SDK_STORE_URL")
        .output()
        .unwrap()
}

fn text(output: &[u8]) -> String {
    String::from_utf8(output.to_vec()).unwrap()
}

#[test]
fn help_entrypoints_and_sections_are_consistent() {
    for args in [vec!["help"], vec!["--help"], vec!["-h"]] {
        let output = run(&args);
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(output.stderr.is_empty());
        let help = text(&output.stdout);
        for expected in [
            "Usage:",
            "Summary:",
            "Global options:",
            "Commands:",
            "help <command>",
            "edit",
            "validate",
            "version",
        ] {
            assert!(help.contains(expected), "Missing {expected} in {help}");
        }
        assert!(!help.contains('\x1b'));
    }
    for command in ["edit", "validate", "version", "help"] {
        let via_help = run(&["help", command]);
        let via_flag = run(&[command, "--help"]);
        assert!(via_help.status.success());
        assert!(via_flag.status.success());
        assert_eq!(via_help.stdout, via_flag.stdout);
        assert!(text(&via_help.stdout).contains(&format!("workshop-tui {command}")));
    }
    let all = run(&["help", "--all"]);
    assert!(all.status.success());
    for usage in ["workshop-tui edit", "workshop-tui validate"] {
        assert!(text(&all.stdout).contains(usage));
    }
    assert!(!text(&all.stdout).contains("workshop-tui status"));
}

#[test]
fn removed_status_has_actionable_failure_and_never_connects_to_a_daemon() {
    for args in [
        vec!["status"],
        vec!["status", "--socket", "/missing/socket"],
        vec!["status", "--daemon-url", "http://127.0.0.1:1"],
        vec!["status", "--help"],
    ] {
        let output = run(&args);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(text(&output.stderr).contains("was removed in 0.2.0"));
        assert!(text(&output.stderr).contains("validate"));
    }
}

#[test]
fn validates_offline_preserves_shorthand_and_respects_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("workshop.yaml");
    let yaml = "name: example\nbase: ubuntu@24.04\n";
    fs::write(&file, yaml).unwrap();
    let file = file.to_str().unwrap();
    for args in [vec!["validate", file], vec![file, "--check"]] {
        let result = run(&args);
        assert!(result.status.success(), "{}", text(&result.stderr));
        assert!(text(&result.stdout).starts_with("Valid Workshop definition: example"));
        assert!(result.stderr.is_empty());
    }
    let quiet = run(&["validate", file, "--quiet"]);
    assert!(quiet.status.success());
    assert!(quiet.stdout.is_empty());
    assert!(quiet.stderr.is_empty());
    assert_eq!(fs::read_to_string(file).unwrap(), yaml);
}

#[test]
fn errors_are_on_stderr_with_usage_and_nonzero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.yaml");
    let invalid = dir.path().join("invalid.yaml");
    fs::write(&invalid, "name: INVALID\nbase: ubuntu@24.04\n").unwrap();
    for args in [
        vec!["validate", missing.to_str().unwrap()],
        vec!["validate", invalid.to_str().unwrap()],
        vec!["edit", "--unknown"],
        vec!["edit", "--theme", "unknown"],
        vec!["help", "unknown"],
    ] {
        let output = run(&args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(text(&output.stderr).contains("Usage:"));
        assert!(text(&output.stderr).contains("error:"));
    }
    assert!(!missing.exists());
}

#[test]
fn snap_missing_paths_explain_confinement_and_home_connection() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_workshop-tui"))
        .args([
            "validate",
            dir.path().join("missing.yaml").to_str().unwrap(),
        ])
        .env("SNAP_NAME", "workshop-config")
        .env("SNAP", "/snap/workshop-config/test")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = text(&output.stderr);
    assert!(error.contains("Cannot read definition"));
    assert!(error.contains("Strict snap access"));
    assert!(error.contains("~/projects/my-workshop"));
    assert!(error.contains("snap connections workshop-config"));
}

#[test]
fn version_has_command_and_flag_forms() {
    let command = run(&["version"]);
    let flag = run(&["--version"]);
    assert!(command.status.success() && flag.status.success());
    assert_eq!(command.stdout, flag.stdout);
    assert!(command.stderr.is_empty());
}

#[test]
fn snap_entrypoint_uses_workshop_config_without_changing_legacy_commands() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("workshop-config");
    symlink(env!("CARGO_BIN_EXE_workshop-tui"), &binary).unwrap();
    for args in [
        vec!["--help"],
        vec!["help", "edit"],
        vec!["help", "--all"],
        vec!["version"],
        vec!["--version"],
    ] {
        let output = Command::new(&binary)
            .args(&args)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        let stdout = text(&output.stdout);
        assert!(stdout.contains("workshop-config"), "{stdout}");
        assert!(!stdout.contains("workshop-tui"), "{stdout}");
    }
    let output = Command::new(&binary)
        .args(["validate", "/nonexistent-workshop-definition.yaml"])
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(text(&output.stderr).contains("workshop-config validate"));
    assert!(!text(&output.stderr).contains("workshop-tui"));
    assert!(text(&run(&["version"]).stdout).starts_with("workshop-tui "));
}
