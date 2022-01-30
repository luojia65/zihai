use std::process::Command;
use std::path::{Path, PathBuf};

fn main() {
    println!("Hello, world!");
    let status = Command::new("qemu-system-riscv64")
        .current_dir(project_root())
        .args(&["-machine", "virt"])
        // .args(&["-bios", "rustsbi-qemu.bin"])
        // .args(&["-kernel", "test-kernel.bin"])
        .args(&["-smp", "8"]) // 8 cores
        .arg("-nographic")
        .status()
        .unwrap();
    println!("{}", status);
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
