//! Embeds the app icon into the Windows executable (no effect elsewhere).
fn main() {
    println!("cargo:rerun-if-changed=assets/padd.rc");
    println!("cargo:rerun-if-changed=assets/padd.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/padd.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the Windows icon");
    }
}
