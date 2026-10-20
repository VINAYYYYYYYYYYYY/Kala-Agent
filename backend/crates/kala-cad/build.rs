//! Deterministic native OpenCascade discovery and build for `kala-cad`.
//!
//! This script never downloads anything and never looks for Python, conda,
//! FreeCAD or OCP. It compiles the narrow C++ FFI adapter against an explicitly
//! configured native OCCT SDK and links the OCCT shared/import libraries.
//!
//! Discovery order (first usable result wins per directory):
//!
//! 1. explicit `OCCT_INCLUDE_DIR` / `OCCT_LIB_DIR` (also accepted under the
//!    `KALA_OCCT_INCLUDE_DIR` / `KALA_OCCT_LIB_DIR` names);
//! 2. an `OCCT_ROOT` / `KALA_OCCT_ROOT` prefix (`include/opencascade`, `inc`,
//!    `lib`, `lib64`, `win64/vc14/lib`, `win64/vc14/bin`, `bin`);
//! 3. an `OpenCASCADE_DIR` CMake package directory, other CMake package
//!    locations, and the `OpenCASCADE_INCLUDE_DIR`/`OpenCASCADE_LIBRARY_DIR`
//!    values inside `OpenCASCADEConfig.cmake`;
//! 4. `pkg-config --cflags/--libs opencascade` on non-Windows hosts;
//! 5. a `VCPKG_ROOT`/`VCPKG_INSTALLATION_ROOT` tree for the selected triplet
//!    (`installed/<triplet>/include/opencascade`, `installed/<triplet>/lib`,
//!    `installed/<triplet>/debug/lib`, `installed/<triplet>/bin`);
//! 6. platform defaults (Linux distribution paths, macOS Homebrew, the
//!    canonical Windows `C:\OpenCASCADE-7.9.0` layout).
//!
//! A missing include header or `TKernel` library is a hard build error. Build
//! without the `native` feature for the explicit unavailable mode.
//!
//! Windows MSVC + vcpkg: use the `x64-windows` triplet (dynamic CRT) and set
//! `VCPKG_ROOT`; the script finds the OCCT tree without extra flags. For a
//! static-CRT triplet set `KALA_CAD_STATIC_CRT=1` so the adapter is compiled
//! with `/MT` to match. At test time the OCCT DLLs live in
//! `%VCPKG_ROOT%\installed\x64-windows\bin`; add that directory to `PATH` or
//! copy the DLLs next to the executable. The packaging owner stages them.

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// OCCT modules used directly or reached by the STEP translator.
const DEFAULT_LIBS: &[&str] = &[
    "TKernel",
    "TKMath",
    "TKG2d",
    "TKG3d",
    "TKGeomBase",
    "TKGeomAlgo",
    "TKTopAlgo",
    "TKBRep",
    "TKPrim",
    "TKBO",
    "TKBool",
    "TKMesh",
    "TKShHealing",
    "TKHLR",
    "TKXSBase",
    "TKDE",
    "TKDESTEP",
    "TKXCAF",
    "TKLCAF",
];

fn main() {
    for name in [
        "OCCT_INCLUDE_DIR",
        "OCCT_LIB_DIR",
        "OCCT_ROOT",
        "KALA_OCCT_INCLUDE_DIR",
        "KALA_OCCT_LIB_DIR",
        "KALA_OCCT_ROOT",
        "KALA_OCCT_LIBS",
        "OpenCASCADE_DIR",
        "VCPKG_ROOT",
        "VCPKG_INSTALLATION_ROOT",
        "VCPKG_TARGET_TRIPLET",
        "KALA_CAD_CXXFLAGS",
        "KALA_CAD_SANITIZE",
        "KALA_CAD_STATIC_CRT",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=src/ffi/adapter.cpp");

    if env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        println!(
            "cargo:warning=kala-cad is built without the native feature: the CAD kernel is unavailable and design_available() is false"
        );
        return;
    }

    let target = env::var("TARGET").unwrap_or_default();
    let host = env::var("HOST").unwrap_or_default();
    let windows = target.contains("windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let discovery = discover(&target, windows);

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("src/ffi/adapter.cpp")
        .include(&discovery.include)
        .warnings(true)
        .define("KALA_CAD_FFI", None);
    if windows {
        build.flag_if_supported("/EHsc");
        build.flag_if_supported("/permissive-");
        build.flag_if_supported("/utf-8");
        build.flag_if_supported("/Zc:__cplusplus");
        // vcpkg's `x64-windows` triplet uses the dynamic CRT. A static
        // triplet must opt in explicitly so the adapter's allocator matches
        // every OCCT DLL/static library it calls.
        let static_crt = env::var("KALA_CAD_STATIC_CRT").is_ok_and(|value| value == "1");
        build.static_crt(static_crt);
    } else {
        build.flag_if_supported("-fvisibility=hidden");
        build.flag_if_supported("-fno-strict-aliasing");
    }
    if let Ok(extra) = env::var("KALA_CAD_CXXFLAGS") {
        for flag in extra.split_whitespace() {
            build.flag(flag);
        }
    }
    if env::var("KALA_CAD_SANITIZE").is_ok_and(|value| value == "address") {
        if windows || host != target {
            panic!(
                "kala-cad: KALA_CAD_SANITIZE=address requires a native non-MSVC build; \
                 use a Debug/ASan MSVC or cross build in the packaging pipeline"
            );
        }
        build
            .flag("-fsanitize=address")
            .flag("-fno-omit-frame-pointer")
            .flag("-g");
        println!("cargo:rustc-link-arg=-fsanitize=address");
        println!("cargo:rustc-link-lib=dylib=asan");
    }
    build.compile("kala_cad_ffi");

    let mut search_dirs = BTreeSet::new();
    for lib in &discovery.lib_dirs {
        search_dirs.insert(lib.clone());
    }
    for dir in &search_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    let libs = match env::var("KALA_OCCT_LIBS") {
        Ok(value) if !value.trim().is_empty() => value
            .split([';', ',', ' '])
            .filter(|item| !item.trim().is_empty())
            .map(|item| item.trim().to_owned())
            .collect::<Vec<_>>(),
        _ => DEFAULT_LIBS.iter().map(|item| (*item).to_owned()).collect(),
    };
    for name in &libs {
        println!("cargo:rustc-link-lib=dylib={name}");
    }

    // Make a non-system SDK usable from the test binaries on Unix without
    // asking the caller to remember LD_LIBRARY_PATH. Packaging owners may
    // strip or replace this with their own runtime deployment; see README.
    if !windows && host == target {
        for lib in &discovery.lib_dirs {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
        }
    }

    if msvc {
        println!(
            "cargo:warning=kala-cad: MSVC build found OCCT at {} and libs {:?}; \
             add the OCCT/vcpkg bin directory to PATH for test execution",
            discovery.include.display(),
            discovery
                .lib_dirs
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        );
    }
}

struct Discovery {
    include: PathBuf,
    lib_dirs: Vec<PathBuf>,
}

fn discover(target: &str, windows: bool) -> Discovery {
    let mut includes = Vec::new();
    let mut libs = Vec::new();

    // 1. Explicit directories (both documented spellings).
    if let Some(path) = env_path("OCCT_INCLUDE_DIR") {
        includes.push(path);
    }
    if let Some(path) = env_path("KALA_OCCT_INCLUDE_DIR") {
        includes.push(path);
    }
    if let Some(path) = env_path("OCCT_LIB_DIR") {
        libs.push(path);
    }
    if let Some(path) = env_path("KALA_OCCT_LIB_DIR") {
        libs.push(path);
    }

    // 2. Prefix roots.
    for name in ["OCCT_ROOT", "KALA_OCCT_ROOT"] {
        if let Some(root) = env_path(name) {
            push_root(&root, &mut includes, &mut libs, windows);
        }
    }

    // 3. CMake package config, including `OpenCASCADE_DIR` and vcpkg.
    for config in cmake_candidates(windows) {
        if let Some((include, library)) = parse_cmake_config(&config) {
            includes.push(include);
            libs.push(library);
        }
    }

    // 4. pkg-config on Unix-like hosts.
    if !windows {
        let (pkg_includes, pkg_libs) = pkg_config_dirs();
        includes.extend(pkg_includes);
        libs.extend(pkg_libs);
    }

    // 5. vcpkg installation trees.
    for root in ["VCPKG_ROOT", "VCPKG_INSTALLATION_ROOT"] {
        if let Some(root) = env_path(root) {
            for triplet in vcpkg_triplets(target, windows) {
                let prefix = root.join("installed").join(&triplet);
                includes.push(prefix.join("include").join("opencascade"));
                includes.push(prefix.join("include"));
                libs.push(prefix.join("lib"));
                libs.push(prefix.join("debug").join("lib"));
                libs.push(prefix.join("bin"));
            }
        }
    }

    // 6. Platform defaults.
    if windows {
        includes.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\include\opencascade"));
        includes.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\inc"));
        includes.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\include"));
        libs.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\win64\vc14\lib"));
        libs.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\win64\vc14\bin"));
    } else {
        includes.push(PathBuf::from("/usr/include/opencascade"));
        includes.push(PathBuf::from("/usr/local/include/opencascade"));
        includes.push(PathBuf::from("/usr/include/occt"));
        includes.push(PathBuf::from("/opt/opencascade/include/opencascade"));
        includes.push(PathBuf::from("/opt/occt/include/opencascade"));
        if target.contains("apple") {
            if let Ok(prefix) = Command::new("brew")
                .args(["--prefix", "opencascade"])
                .output()
            {
                if prefix.status.success() {
                    let root = PathBuf::from(String::from_utf8_lossy(&prefix.stdout).trim());
                    includes.push(root.join("include").join("opencascade"));
                    libs.push(root.join("lib"));
                }
            }
        }
        libs.push(PathBuf::from("/usr/lib"));
        libs.push(PathBuf::from("/usr/lib64"));
        libs.push(PathBuf::from("/usr/local/lib"));
        libs.push(PathBuf::from("/usr/local/lib64"));
        libs.push(PathBuf::from("/opt/opencascade/lib"));
        libs.push(PathBuf::from("/opt/occt/lib"));
    }

    dedupe(&mut includes);
    dedupe(&mut libs);

    let include = includes
        .iter()
        .find(|path| path.join("Standard_Version.hxx").is_file())
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "kala-cad: native OpenCascade headers were not found. Set OCCT_INCLUDE_DIR to \
             the directory containing Standard_Version.hxx (for example \
             /usr/include/opencascade), or OCCT_ROOT / OpenCASCADE_DIR / VCPKG_ROOT. Tried: \
             {}. Build with --no-default-features for the explicit unavailable mode.",
                display_list(&includes)
            )
        });

    let probe = kernel_probe(target, windows);
    let usable_libs = libs
        .iter()
        .filter(|path| has_kernel_library(path, &probe))
        .cloned()
        .collect::<Vec<_>>();
    if usable_libs.is_empty() {
        panic!(
            "kala-cad: native OpenCascade import libraries were not found. Set OCCT_LIB_DIR \
             to the directory containing {probe}, or OCCT_ROOT / OpenCASCADE_DIR / \
             VCPKG_ROOT. Tried: {}. Build with --no-default-features for the explicit \
             unavailable mode.",
            display_list(&libs)
        );
    }

    Discovery {
        include,
        lib_dirs: usable_libs,
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn push_root(root: &Path, includes: &mut Vec<PathBuf>, libs: &mut Vec<PathBuf>, windows: bool) {
    includes.push(root.join("include").join("opencascade"));
    includes.push(root.join("inc"));
    includes.push(root.join("include"));
    libs.push(root.join("lib"));
    libs.push(root.join("lib64"));
    if windows {
        libs.push(root.join("win64").join("vc14").join("lib"));
        libs.push(root.join("win64").join("vc14").join("bin"));
        libs.push(root.join("bin"));
    }
}

fn cmake_candidates(windows: bool) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env_path("OpenCASCADE_DIR") {
        dirs.push(dir.clone());
        dirs.push(dir.join("cmake"));
        dirs.push(dir.join("lib").join("cmake").join("opencascade"));
    }
    for name in ["OCCT_ROOT", "KALA_OCCT_ROOT"] {
        if let Some(root) = env_path(name) {
            dirs.push(root.join("lib").join("cmake").join("opencascade"));
            dirs.push(root.join("lib64").join("cmake").join("opencascade"));
            dirs.push(root.join("cmake"));
        }
    }
    for name in ["VCPKG_ROOT", "VCPKG_INSTALLATION_ROOT"] {
        if let Some(root) = env_path(name) {
            for triplet in vcpkg_triplets("", windows) {
                dirs.push(
                    root.join("installed")
                        .join(&triplet)
                        .join("share")
                        .join("opencascade"),
                );
                dirs.push(
                    root.join("installed")
                        .join(&triplet)
                        .join("lib")
                        .join("cmake")
                        .join("opencascade"),
                );
            }
        }
    }
    if windows {
        dirs.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\cmake"));
        dirs.push(PathBuf::from(r"C:\OpenCASCADE-7.9.0\lib\cmake\opencascade"));
    } else {
        dirs.push(PathBuf::from("/usr/lib/cmake/opencascade"));
        dirs.push(PathBuf::from("/usr/lib64/cmake/opencascade"));
        dirs.push(PathBuf::from("/usr/local/lib/cmake/opencascade"));
        dirs.push(PathBuf::from("/usr/local/lib64/cmake/opencascade"));
        dirs.push(PathBuf::from("/opt/opencascade/lib/cmake/opencascade"));
        dirs.push(PathBuf::from("/opt/occt/lib/cmake/opencascade"));
    }
    dirs.into_iter()
        .map(|dir| {
            if dir
                .extension()
                .is_some_and(|extension| extension == "cmake")
            {
                dir
            } else {
                dir.join("OpenCASCADEConfig.cmake")
            }
        })
        .collect()
}

fn parse_cmake_config(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let text = std::fs::read_to_string(path).ok()?;
    Some((
        cmake_value(&text, "OpenCASCADE_INCLUDE_DIR")?,
        cmake_value(&text, "OpenCASCADE_LIBRARY_DIR")?,
    ))
}

fn cmake_value(text: &str, name: &str) -> Option<PathBuf> {
    for line in text.lines() {
        let trimmed = line.trim();
        let head = trimmed
            .strip_prefix("set")
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix('('))
            .map(str::trim_start);
        let Some(rest) = head else {
            continue;
        };
        let Some(value) = rest.strip_prefix(name) else {
            continue;
        };
        if !value.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let value = value.trim_start();
        let value = value.strip_prefix('"').unwrap_or(value);
        let value = value.split('"').next()?.trim();
        if value.is_empty() {
            return None;
        }
        return Some(PathBuf::from(value));
    }
    None
}

fn pkg_config_dirs() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let available = Command::new("pkg-config")
        .args(["--exists", "opencascade"])
        .status()
        .is_ok_and(|status| status.success());
    if !available {
        return (Vec::new(), Vec::new());
    }
    let cflags = Command::new("pkg-config")
        .args(["--cflags", "opencascade"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let libs = Command::new("pkg-config")
        .args(["--libs", "opencascade"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let mut includes = Vec::new();
    let mut directories = Vec::new();
    let mut tokens = cflags.split_whitespace();
    while let Some(token) = tokens.next() {
        if let Some(path) = token.strip_prefix("-I") {
            if !path.is_empty() {
                includes.push(PathBuf::from(path));
            }
        } else if token == "-isystem" {
            if let Some(path) = tokens.next() {
                includes.push(PathBuf::from(path));
            }
        }
    }
    for token in libs.split_whitespace() {
        if let Some(path) = token.strip_prefix("-L") {
            if !path.is_empty() {
                directories.push(PathBuf::from(path));
            }
        }
    }
    (includes, directories)
}

fn vcpkg_triplets(target: &str, windows: bool) -> Vec<String> {
    if let Ok(value) = env::var("VCPKG_TARGET_TRIPLET") {
        if !value.trim().is_empty() {
            return vec![value.trim().to_owned()];
        }
    }
    if windows {
        if target.starts_with("aarch64") {
            vec!["arm64-windows".to_owned()]
        } else if target.starts_with("i686") {
            vec!["x86-windows".to_owned()]
        } else {
            vec!["x64-windows".to_owned()]
        }
    } else if target.contains("apple") {
        vec!["x64-osx".to_owned(), "arm64-osx".to_owned()]
    } else if target.starts_with("aarch64") {
        vec!["arm64-linux".to_owned()]
    } else {
        vec!["x64-linux".to_owned()]
    }
}

fn kernel_probe(target: &str, windows: bool) -> String {
    if windows {
        "TKernel.lib".to_owned()
    } else if target.contains("apple") {
        "libTKernel.dylib".to_owned()
    } else {
        "libTKernel.so".to_owned()
    }
}

fn has_kernel_library(dir: &Path, probe: &str) -> bool {
    if dir.join(probe).is_file() {
        return true;
    }
    if probe.ends_with(".so") {
        // Some distributions keep only versioned shared objects without the
        // development symlink; accept the module by prefix but keep the search
        // deterministic.
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut names = entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name.starts_with("libTKernel.so"))
                .collect::<Vec<_>>();
            names.sort();
            return !names.is_empty();
        }
    }
    false
}

fn dedupe(paths: &mut Vec<PathBuf>) {
    let mut seen = BTreeSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn display_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}
