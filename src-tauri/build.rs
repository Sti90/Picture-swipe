fn main() {
    println!("cargo:rerun-if-changed=src/photokit/bridge.m");
    cc::Build::new()
        .file("src/photokit/bridge.m")
        .flag("-fobjc-arc")
        .compile("photokit_bridge");
    for framework in ["Foundation", "AppKit", "CoreGraphics", "ImageIO", "Photos"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    tauri_build::build()
}
