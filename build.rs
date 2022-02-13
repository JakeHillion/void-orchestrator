use std::process::Command;

fn main() {
    let output = Command::new("git")
        .args(&["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let mut git_hash = String::from_utf8(output.stdout).unwrap();
    git_hash.truncate(16);
    let clean_status = Command::new("git")
        .args(&["diff", "--exit-code"])
        .status()
        .unwrap();
    if !clean_status.success() {
        git_hash.push_str("-dirty");
    }
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
}
