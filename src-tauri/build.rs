fn main() {
    tauri_build::build();

    // Recompile when the ObjC bridge changes
    println!("cargo:rerun-if-changed=src/screencapture_bridge.m");

    // Compile the Objective-C ScreenCaptureKit bridge
    cc::Build::new()
        .file("src/screencapture_bridge.m")
        .flag("-fobjc-arc")
        .compile("screencapture_bridge");

    // Link required macOS frameworks
    println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    println!("cargo:rustc-link-lib=framework=CoreMedia");
    println!("cargo:rustc-link-lib=framework=CoreAudio");
    println!("cargo:rustc-link-lib=framework=AudioToolbox");
}
