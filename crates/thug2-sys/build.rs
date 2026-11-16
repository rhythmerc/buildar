//! Builds the THUG2 engine's static libraries with CMake from two checkouts (none of it, nor any game file, is in
//! this repository):
//! - `THUG2_HEADLESS_DIR`: thug2-headless, the harness that runs the decomp without a window (its CMakeLists.txt,
//!   backends and the C API, app/thug2_engine.h; github.com/rhythmerc/thug2-headless). Default: `../thug2-headless`
//!   beside this workspace.
//! - `THUG2_DECOMP_DIR`: thug2-decomp with the Linux/Android patches (github.com/rhythmerc/thug2-decomp, branch
//!   `linux-headless`; upstream github.com/CanuckAussieKev/thug2-decomp).
//!   Default: `../thug2-decomp` beside this workspace.
//!
//! Android (aarch64-linux-android): the NDK's CMake toolchain (`ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT`), API 32, the
//! shared C++ runtime (libc++_shared.so, which the app already ships for Bevy's audio). Elsewhere: clang (`THUG2_CXX`,
//! default `clang++-18` if there is one, else `clang++`) and the system's C++ runtime.
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let beside = manifest.join("../../..");
    let dir = |var: &str, default: &str| -> PathBuf {
        println!("cargo:rerun-if-env-changed={var}");
        std::env::var_os(var).map(PathBuf::from).unwrap_or_else(|| beside.join(default))
    };
    let headless = dir("THUG2_HEADLESS_DIR", "thug2-headless");
    let decomp = dir("THUG2_DECOMP_DIR", "thug2-decomp");
    for (what, path, file) in [
        ("THUG2_HEADLESS_DIR", &headless, "CMakeLists.txt"),
        ("THUG2_DECOMP_DIR", &decomp, "src"),
    ] {
        if !path.join(file).exists() {
            panic!("thug2-sys: no {} at {} (set {what})", file, path.display());
        }
    }
    for watched in ["CMakeLists.txt", "build_sources.txt", "app", "backends", "world", "compat"] {
        println!("cargo:rerun-if-changed={}", headless.join(watched).display());
    }
    println!("cargo:rerun-if-changed={}", decomp.join("src").display());

    let target = std::env::var("TARGET").unwrap();
    let android = target.contains("android");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("cmake");
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(&headless)
        .arg("-B")
        .arg(&out)
        .arg("-G")
        .arg("Ninja")
        .arg("-DCMAKE_BUILD_TYPE=RelWithDebInfo")
        .arg("-DTHUG2_DRIVE=OFF")
        .arg(format!("-DTHUG2_DECOMP_DIR={}", decomp.display()));
    if android {
        println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
        println!("cargo:rerun-if-env-changed=ANDROID_NDK_ROOT");
        let ndk = std::env::var_os("ANDROID_NDK_HOME")
            .or_else(|| std::env::var_os("ANDROID_NDK_ROOT"))
            .map(PathBuf::from)
            .expect("thug2-sys: set ANDROID_NDK_HOME for an Android build");
        let abi = if target.starts_with("aarch64") { "arm64-v8a" } else { panic!("thug2-sys: Android is arm64 only") };
        configure
            .arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", ndk.join("build/cmake/android.toolchain.cmake").display()))
            .arg(format!("-DANDROID_ABI={abi}"))
            .arg("-DANDROID_PLATFORM=android-32")
            .arg("-DANDROID_STL=c++_shared");
    } else {
        println!("cargo:rerun-if-env-changed=THUG2_CXX");
        let cxx = std::env::var("THUG2_CXX").unwrap_or_else(|_| {
            if which("clang++-18") { "clang++-18".into() } else { "clang++".into() }
        });
        let cc = cxx.replace("clang++", "clang");
        configure.arg(format!("-DCMAKE_CXX_COMPILER={cxx}")).arg(format!("-DCMAKE_C_COMPILER={cc}"));
    }
    if !out.join("CMakeCache.txt").exists() {
        run(&mut configure, "configure");
    }
    let jobs = std::env::var("NUM_JOBS").unwrap_or_else(|_| "8".into());
    run(
        Command::new("cmake")
            .arg("--build")
            .arg(&out)
            .args(["--target", "thug2_engine", "-j", &jobs]),
        "build",
    );

    println!("cargo:rustc-link-search=native={}", out.display());
    // (the engine first: it calls into the core, which calls into lzham)
    for lib in ["thug2_engine", "thug2_core", "lzham"] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
    if android {
        println!("cargo:rustc-link-lib=dylib=c++_shared");
        println!("cargo:rustc-link-lib=dylib=log");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=dl");
        println!("cargo:rustc-link-lib=dylib=pthread");
    }
    println!("cargo:rustc-link-lib=dylib=m");
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|p| p.join(program).is_file()))
}

fn run(command: &mut Command, what: &str) {
    let status = command.status().unwrap_or_else(|e| panic!("thug2-sys: cmake ({what}) didn't start: {e}"));
    if !status.success() {
        panic!("thug2-sys: cmake {what} failed ({status})");
    }
}
