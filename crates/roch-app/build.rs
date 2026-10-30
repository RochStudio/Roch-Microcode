//! Embed the application icon, version information and manifest in RochMicrocode.exe.

fn main() {
    println!("cargo:rerun-if-changed=../../assets/RochMicrocode.ico");
    println!("cargo:rerun-if-changed=../../assets/verification-bundle.zip");
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/RochMicrocode.ico")
        .set("ProductName", "Roch Microcode")
        .set("FileDescription", "Intel BIOS microcode transfer, independent verification and USB preparation")
        .set("LegalCopyright", "Copyright (C) 2026 Roch Studio. GPL-3.0-or-later.")
        .set("OriginalFilename", "RochMicrocode.exe")
        .set("InternalName", "RochMicrocode")
        .set_manifest_file("app.manifest");
    if let Err(e) = res.compile() {
        // A missing resource compiler only loses the embedded icon/manifest; the
        // window icon is still set at runtime. Release CI checks the icon exists.
        println!("cargo:warning=Windows resources were not embedded: {e}");
    }
}
