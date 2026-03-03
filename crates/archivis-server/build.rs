use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=ARCHIVIS_VERSION");

    let version = std::env::var("ARCHIVIS_VERSION")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(git_describe)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    println!("cargo:rustc-env=ARCHIVIS_VERSION={version}");
}

fn git_describe() -> Option<String> {
    Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
