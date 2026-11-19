fn main() {
    // Embarque le manifeste « requireAdministrator » et l'icône (razdisk.rc) dans l'exécutable Windows.
    println!("cargo:rerun-if-changed=razdisk.rc");
    println!("cargo:rerun-if-changed=razdisk.exe.manifest");
    println!("cargo:rerun-if-changed=icon.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let _ = embed_resource::compile("razdisk.rc", embed_resource::NONE);
    }
}
