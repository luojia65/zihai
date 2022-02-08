use clap::{Parser, Subcommand};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

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
    /// Emulate in QEMU under debug configuration
    Debug {},
    /// Run GDB debugger
    Gdb {},
}

fn main() {
    let args = Cli::parse();

    match &args.command {
        Commands::Make {} => {
            println!("xtask: make hypervisor");
            xtask_build_zihai();
        }
        Commands::Qemu {} => {
            println!("xtask: make hypervisor and run in QEMU");
            xtask_build_zihai();
            xtask_run_zihai();
        }
        Commands::Debug {} => {
            println!("xtask: make hypervisor and debug in QEMU");
            xtask_build_zihai();
            xtask_debug_zihai();
        }
        Commands::Gdb {} => {
            println!("xtask: debug hypervisor on GDB server localhost:3333");
            xtask_gdb_zihai();
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
        eprintln!("xtask: cargo build failed with {}", status);
        process::exit(1);
    }
}

fn xtask_run_zihai() {
    let mut command = Command::new("qemu-system-riscv64");
    command.current_dir(project_root());
    command.args(&["-cpu", "rv64,x-h=true"]); // enable hypervisor
    command.args(&["-machine", "virt"]);
    command.args(&["-bios", "bootloader/rustsbi-qemu.bin"]);
    // QEMU supports to run ELF file directly
    command.args(&["-kernel", "target/riscv64imac-unknown-none-elf/debug/zihai"]);
    command.args(&["-smp", "8"]); // 8 cores
    command.arg("-nographic");

    let status = command.status().expect("run program");

    if !status.success() {
        eprintln!("xtask: qemu failed with {}", status);
        process::exit(status.code().unwrap_or(1));
    }
}

fn xtask_debug_zihai() {
    let mut command = Command::new("qemu-system-riscv64");
    command.current_dir(project_root());
    command.args(&["-cpu", "rv64,x-h=true"]); // enable hypervisor
    command.args(&["-machine", "virt"]);
    command.args(&["-bios", "bootloader/rustsbi-qemu.bin"]);
    command.args(&["-kernel", "target/riscv64imac-unknown-none-elf/debug/zihai"]);
    command.args(&["-smp", "8"]); // 8 cores
    command.args(&["-gdb", "tcp::3333"]);
    command.arg("-S"); // freeze CPU at startup
    command.arg("-nographic");

    let status = command.status().expect("run program");

    if !status.success() {
        eprintln!("xtask: qemu failed with {}", status);
        process::exit(status.code().unwrap_or(1));
    }
}

fn xtask_gdb_zihai() {
    let mut command = Command::new("riscv64-unknown-elf-gdb");
    command.current_dir(project_root());
    command.args(&[
        "--eval-command",
        "file target/riscv64imac-unknown-none-elf/debug/zihai",
    ]);
    command.args(&["--eval-command", "target extended-remote localhost:3333"]);
    command.arg("--quiet");

    ctrlc::set_handler(move || {
        // when ctrl-c, don't exit gdb
    })
    .expect("disable Ctrl-C exit");

    let status = command.status().expect("run program");

    if !status.success() {
        eprintln!("xtask: gdb failed with {}", status);
        process::exit(status.code().unwrap_or(1));
    }
}

fn project_root() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(1)
        .unwrap()
        .to_path_buf()
}
