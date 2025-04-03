use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let asm_dir = Path::new("asm");
    let build_dir = Path::new("build/asm");

    fs::create_dir_all(build_dir).expect("Failed to create build/asm directory");

    let asm_sources: Vec<PathBuf> = fs::read_dir(asm_dir)
        .expect("Failed to read asm directory")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? == "s" {
                Some(path)
            } else {
                None
            }
        })
        .collect();

    if asm_sources.is_empty() {
        println!("cargo:warning=No assembly source files found in asm/");
        return;
    }

    for source in &asm_sources {
        let source_file = source.to_str().unwrap();
        let base_name = source.file_stem().unwrap().to_string_lossy();
        let obj_file = format!("{}/{}.o", out_dir, base_name);
        let cfg_file = format!("asm/{}.cfg", base_name);
        let bin_file = format!("{}/{}.bin", build_dir.display(), base_name);

        println!("cargo:rerun-if-changed={}", source_file);

        let status = Command::new("ca65")
            .arg(source_file)
            .arg("-o")
            .arg(&obj_file)
            .status()
            .expect("Failed to run ca65 assembler");

        if !status.success() {
            panic!("ca65 assembly failed for {}", source_file);
        }

        let status = Command::new("ld65")
            .arg("-C")
            .arg(&cfg_file)
            .arg(&obj_file)
            .arg("-o")
            .arg(&bin_file)
            .status()
            .expect("Failed to run ld65 linker");

        if !status.success() {
            panic!("ld65 linking failed for {}", source_file);
        }

        println!("Built: {} -> {}", source_file, bin_file);
    }
}
