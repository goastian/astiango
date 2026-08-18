// AstianGO is an open source web search engine.
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::process::Command;

fn run_astiango(argument: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_astiango"))
        .arg(argument)
        .output()
        .expect("AstianGO CLI should run");

    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("CLI output should be UTF-8")
}

#[test]
fn version_uses_astiango_command_name() {
    assert!(run_astiango("--version").starts_with("astiango "));
}

#[test]
fn help_uses_astiango_branding() {
    let help = run_astiango("--help");

    assert!(help.contains("Usage: astiango <COMMAND>"));
    assert!(!help.to_ascii_lowercase().contains("stract"));
}
