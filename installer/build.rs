// Embeds the app manifest, and passes the built app exe and its version to the installer.
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();

    let manifest = root.join("app.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());

    let app = root.join("target").join("release").join("unshufflz.exe");
    if !app.exists() {
        panic!("{} not found. Run `cargo build --release` first.", app.display());
    }
    println!("cargo:rerun-if-changed={}", app.display());
    println!("cargo:rustc-env=APP_EXE={}", app.display());

    // Version of the app = version in the root Cargo.toml [package].
    let toml_path = root.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", toml_path.display());
    let toml = std::fs::read_to_string(&toml_path).unwrap();
    let version = toml
        .lines()
        .find_map(|l| l.trim().strip_prefix("version = \"")?.strip_suffix('"').map(str::to_string))
        .expect("version in Cargo.toml");
    println!("cargo:rustc-env=APP_VERSION={}", version);
}
