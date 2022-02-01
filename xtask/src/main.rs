use std::process::{self, Command};
use std::path::{Path, PathBuf};
use std::env;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[clap(name = "xtask")]
#[clap(about = "Program that help you build and debug zihai hypervisor", long_about = None)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Build ELF and binary for hypervisor
    Make {},
    /// Emulate hypervisor system in QEMU
    Qemu {},
}

fn main() {
    let args = Cli::parse();

    match &args.command {
        Commands::Make { } => {
            println!("Make hypervisor");
            xtask_build_zihai();
        }
        Commands::Qemu { } => {
            println!("Make hypervisor and run in QEMU");
            xtask_build_zihai();
            xtask_run_zihai();
        }
    }
}

const DEFAULT_TARGET: &'static str = "riscv64imac-unknown-none-elf";

fn xtask_build_zihai() {
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut command = Command::new(cargo);
    command.current_dir(project_root().join("zihai"));
    command.arg("build");
    command.args(&["--package", "zihai"]);
    command.args(&["--target", DEFAULT_TARGET]);
    let status = command.status().unwrap();
    if !status.success() {
        eprintln!("cargo build failed");
        process::exit(1);
    }
}

fn xtask_run_zihai() {
    // run ELF file
    let status = Command::new("qemu-system-riscv64")
        .current_dir(project_root())
        .args(&["-machine", "virt"])
        .args(&["-bios", "bootloader/rustsbi-qemu.bin"])
        .args(&["-kernel", "target/riscv64imac-unknown-none-elf/debug/zihai"])
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
