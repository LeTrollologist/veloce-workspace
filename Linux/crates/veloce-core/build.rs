/*!
Build script for veloce-core.

On Windows:
- Embeds `windows.manifest` so the binary requests Administrator elevation via UAC.
- Links the Windows service API.

On other platforms: no-op (cross-compile stubs only).
*/

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=windows.manifest");

    #[cfg(target_os = "windows")]
    embed_manifest();
}

#[cfg(target_os = "windows")]
fn embed_manifest() {
    // Embed the application manifest so Windows knows this binary requires
    // Administrator privileges (necessary for service install/Job Objects).
    //
    // The `winres` crate handles the RC compilation and linking automatically.
    // If winres is unavailable, fall back to a raw link directive.
    // CARGO_MANIFEST_DIR is the crate root — always use an absolute path so the
    // linker can find the file regardless of where Cargo invokes link.exe from.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let manifest_path = std::path::Path::new(&manifest_dir).join("windows.manifest");
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "msvc" && manifest_path.exists() {
        // Embed compatibility + longPathAware sections from our manifest file.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
        // Set UAC level separately to avoid merge conflict with the linker's
        // auto-generated default manifest fragment.
        println!("cargo:rustc-link-arg=/MANIFESTUAC:level='requireAdministrator' uiAccess='false'");
    }

    // Link Windows service / security libraries not auto-linked by the crates
    println!("cargo:rustc-link-lib=advapi32");
    println!("cargo:rustc-link-lib=kernel32");
    println!("cargo:rustc-link-lib=userenv");
    println!("cargo:rustc-link-lib=ntdll");
}