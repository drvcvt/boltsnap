fn main() {
    println!("cargo:rerun-if-changed=assets/windows/boltsnap.rc");
    println!("cargo:rerun-if-changed=assets/windows/boltsnap.manifest");
    println!("cargo:rerun-if-changed=assets/windows/boltsnap.ico");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        embed_resource::compile("assets/windows/boltsnap.rc", embed_resource::NONE)
            .manifest_required()
            .expect("compile Boltsnap Windows manifest");
    }
}
