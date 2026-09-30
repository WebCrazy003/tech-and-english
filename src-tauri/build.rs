fn main() {
    // Shown in About and in the diagnostics (P6).
    let run = |cmd: &str, args: &[&str]| {
        std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let commit = run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into());
    let date = run("date", &["-u", "+%Y-%m-%d"]).unwrap_or_else(|| "0000-00-00".into());
    println!("cargo:rustc-env=TE_COMMIT={commit}");
    println!("cargo:rustc-env=TE_BUILD_DATE={date}");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");
    tauri_build::build()
}
