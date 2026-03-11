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
    let manifest_path = std::path::Path::new("windows.manifest");
    if manifest_path.exists() {
        // Tell the linker to embed the manifest
        println!(
            "cargo:rustc-link-arg=/MANIFEST:EMBED"
        );
        println!(
            "cargo:rustc-link-arg=/MANIFESTINPUT:windows.manifest"
        );
    }

    // Link Windows service / security libraries not auto-linked by the crates
    println!("cargo:rustc-link-lib=advapi32");
    println!("cargo:rustc-link-lib=kernel32");
    println!("cargo:rustc-link-lib=userenv");
    println!("cargo:rustc-link-lib=ntdll");
}