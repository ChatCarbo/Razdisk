fn main() {
    println!("cargo:rerun-if-changed=razdisk.exe.manifest");
    println!("cargo:rerun-if-changed=razdisk.rc");
    embed_resource::compile("razdisk.rc", embed_resource::NONE);
}