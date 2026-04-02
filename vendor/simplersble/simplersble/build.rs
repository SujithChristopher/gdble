use cmake;
use cxx_build;
use std::env;
use std::path::Path;

fn compile_simpleble() {
    let build_debug = env::var("DEBUG").unwrap() == "true";
    if build_debug { println!("cargo:warning=Building in DEBUG mode"); }

    let cargo_manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let simplersble_source_path = Path::new(&cargo_manifest_dir).join("simplersble");
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    // println!("cargo:warning=All Environment Variables:");
    // println!("cargo:warning=CWD: {}", env::current_dir().unwrap().display());
    // for (key, value) in env::vars() {
    //     println!("cargo:warning=ENV: {} - {}", key, value);
    // }

    // The simpleble library name depends if we're building in debug more or not.
    let simpleble_library_name = if build_debug {"simpleble-debug"} else {"simpleble"};
    let mut simpleble_config = cmake::Config::new("simpleble");
    if target_os == "android" {
        let android_abi = env::var("ANDROID_ABI").unwrap_or_else(|_| match target_arch.as_str() {
            "aarch64" => "arm64-v8a".to_string(),
            "arm" => "armeabi-v7a".to_string(),
            "x86" => "x86".to_string(),
            "x86_64" => "x86_64".to_string(),
            _ => panic!("Unexpected Android target architecture"),
        });
        let android_platform =
            env::var("ANDROID_PLATFORM").unwrap_or_else(|_| "android-31".to_string());
        let android_api = android_platform.trim_start_matches("android-");

        simpleble_config
            .define("CMAKE_SYSTEM_NAME", "Android")
            .define("ANDROID_ABI", &android_abi)
            .define("ANDROID_PLATFORM", &android_platform)
            .define("CMAKE_SYSTEM_VERSION", android_api);
    }

    let simpleble_build_dest = simpleble_config.build();
    let simpleble_include_path = Path::new(&simpleble_build_dest).join("include");

    cxx_build::CFG.exported_header_dirs.push(&simpleble_include_path);
    cxx_build::CFG.exported_header_dirs.push(&simplersble_source_path);

    println!("cargo:rustc-link-search=native={}/lib", simpleble_build_dest.display());
    println!("cargo:rustc-link-lib=static={}", simpleble_library_name);

    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=framework=Foundation");
            println!("cargo:rustc-link-lib=framework=CoreBluetooth");
        },
        "windows" => {},
        "linux" => {
            println!("cargo:rustc-link-lib=dbus-1");
        },
        "android" => {
            println!("cargo:rustc-link-lib=android");
            println!("cargo:rustc-link-lib=log");
            println!("cargo:rustc-link-lib=nativehelper");
        },
        &_ => panic!("Unexpected target OS")
    }
}

fn main() {
    // TODO: Add all files that would trigger a rerun
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/bindings/Bindings.hpp");
    println!("cargo:rerun-if-changed=src/bindings/Bindings.cpp");
    println!("cargo:rerun-if-env-changed=ANDROID_ABI");
    println!("cargo:rerun-if-env-changed=ANDROID_PLATFORM");

    compile_simpleble();

    if std::env::var("DOCS_RS").is_ok() {
        println!("cargo:warning=Building DOCS");
    }

    // Build the bindings
    cxx_build::bridge("simplersble/src/lib.rs")
        .file("simplersble/src/bindings/Bindings.cpp")
        .flag_if_supported("-std=c++17")
        .compile("simpleble_bindings");
}
