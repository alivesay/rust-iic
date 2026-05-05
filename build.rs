use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    let asm_dir = Path::new("asm");
    let build_dir = Path::new("build/asm");
    let require = env::var("RUST_IIC_REQUIRE_CA65").is_ok();

    fs::create_dir_all(build_dir).expect("Failed to create build/asm directory");

    // Top-level modules: asm/<name>.s + asm/<name>.cfg -> build/asm/<name>.bin
    let mut asm_sources: Vec<(PathBuf, PathBuf)> = fs::read_dir(asm_dir)
        .expect("Failed to read asm directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "s" {
                return None;
            }
            let base = path.file_stem()?.to_string_lossy().into_owned();
            let cfg = asm_dir.join(format!("{base}.cfg"));
            Some((path, cfg))
        })
        .collect();

    // Program projects: asm/programs/<name>/<name>.s + asm/programs/<name>/<name>.cfg
    // -> build/asm/<name>.bin (same flat output dir as top-level modules).
    let programs_dir = asm_dir.join("programs");
    if programs_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&programs_dir) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if !dir.is_dir() {
                    continue;
                }
                let name = match dir.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let src = dir.join(format!("{name}.s"));
                let cfg = dir.join(format!("{name}.cfg"));
                if src.is_file() && cfg.is_file() {
                    asm_sources.push((src, cfg));
                }
            }
        }
    }

    if asm_sources.is_empty() {
        println!("cargo:warning=No assembly source files found in asm/");
        return;
    }

    let have_ca65 = which("ca65") && which("ld65");

    for (source, cfg_file) in &asm_sources {
        let base_name = source.file_stem().unwrap().to_string_lossy().into_owned();
        let bin_file = build_dir.join(format!("{base_name}.bin"));

        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-changed={}", cfg_file.display());

        let needs_rebuild = match (mtime(&bin_file), mtime(source), mtime(cfg_file)) {
            (Some(b), Some(s), c_opt) => {
                let cfg_newer = c_opt.map(|c| c > b).unwrap_or(false);
                s > b || cfg_newer
            }
            _ => true, // bin missing
        };

        if !needs_rebuild {
            continue;
        }

        if !have_ca65 {
            let msg = format!(
                "asm: {} needs rebuilding but ca65/ld65 are not on PATH; using prebuilt {} (install cc65 to regenerate, or run `make -C asm`)",
                source.display(),
                bin_file.display()
            );
            if require || !bin_file.exists() {
                panic!("{msg}");
            }
            println!("cargo:warning={msg}");
            continue;
        }

        let out_dir = env::var("OUT_DIR").unwrap();
        let obj_file = format!("{out_dir}/{base_name}.o");

        let status = Command::new("ca65")
            .arg("-I")
            .arg(asm_dir.join("lib"))
            .arg(source)
            .arg("-o")
            .arg(&obj_file)
            .status()
            .expect("Failed to run ca65 assembler");
        if !status.success() {
            panic!("ca65 assembly failed for {}", source.display());
        }

        let status = Command::new("ld65")
            .arg("-C")
            .arg(cfg_file)
            .arg(&obj_file)
            .arg("-o")
            .arg(&bin_file)
            .status()
            .expect("Failed to run ld65 linker");
        if !status.success() {
            panic!("ld65 linking failed for {}", source.display());
        }

        println!(
            "cargo:warning=asm: rebuilt {} -> {}",
            source.display(),
            bin_file.display()
        );
    }
}

fn which(cmd: &str) -> bool {
    let path = match env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    env::split_paths(&path).any(|dir| {
        let p = dir.join(cmd);
        p.is_file()
            || p.with_extension("exe").is_file()
    })
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}
