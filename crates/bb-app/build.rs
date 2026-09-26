use std::path::PathBuf;

fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("lang");
    slint_build::compile_with_config("ui/app.slint", config).expect("failed to compile Slint UI");

    embed_resource::compile("assets/breakbar.rc", embed_resource::NONE)
        .manifest_required()
        .expect("failed to embed Windows resources");
    embed_resource::compile(write_version_resource(), embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the version information");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed=lang");
}

/// Writes the `VERSIONINFO` resource (what Explorer's file properties and the task manager show)
/// with the version of the crate, and returns its path.
fn write_version_resource() -> PathBuf {
    let env = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"));
    let (major, minor, patch) = (
        env("CARGO_PKG_VERSION_MAJOR"),
        env("CARGO_PKG_VERSION_MINOR"),
        env("CARGO_PKG_VERSION_PATCH"),
    );
    let version = format!("{major}.{minor}.{patch}");
    let numbers = format!("{major},{minor},{patch},0");

    // The numbers are 0x40004 = VOS_NT_WINDOWS32 and 1 = VFT_APP, spelled out so that the file
    // needs no include of `winver.h`.
    let resource = format!(
        r#"1 VERSIONINFO
FILEVERSION {numbers}
PRODUCTVERSION {numbers}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "breakbar.cc"
            VALUE "FileDescription", "Breakbar Launcher"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "breakbar-launcher"
            VALUE "LegalCopyright", "Copyright (c) Patrick Schmidt. MIT License."
            VALUE "OriginalFilename", "breakbar-launcher.exe"
            VALUE "ProductName", "Breakbar Launcher"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );
    let rc_path = PathBuf::from(env("OUT_DIR")).join("version.rc");
    std::fs::write(&rc_path, resource).expect("failed to write the version resource");
    rc_path
}
