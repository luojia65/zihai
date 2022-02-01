#![feature(asm_sym, asm_const, naked_functions)]
#![no_std]
#![no_main]

use core::arch::asm;
use core::mem::MaybeUninit;

// boot hart start
pub extern "C" fn rust_init(hartid: usize, _opaque: usize) {
    // boot hart init
    unsafe {
        asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) hartid + b'A' as usize);
    }
    // call sbi remote retentive suspension, use sbi 0.3 to wake other harts
    unsafe { asm!("li a7, 0x08", "ecall") }; // shutdown // todo: remove
}

pub extern "C" fn rust_init_harts() {
    // join working queue, ...
}

#[panic_handler]
fn on_panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

const BOOT_STACK_SIZE: usize = 64 * 1024; // 64KB
static BOOT_STACK: MaybeUninit<[u8; BOOT_STACK_SIZE]> = MaybeUninit::uninit();

#[link_section = ".text.entry"]
#[export_name = "_start"]
#[naked]
pub unsafe extern "C" fn start() -> ! {
    asm!(
        // start boot hart only
        "beqz   a0, 2f",
        "1:",
        "wfi",
        "j      1b", // non-boot hart, halt
        "2:",
        "mv     t0, a0",
        "mv     t1, a1",
        // detect SBI version
        "li     a7, 0x10", // function id
        "li     a6, 0x0", // get spec version
        "ecall", // call SBI environment
        "li     t2, 0x00000003",
        "blt    a1, t2, 1f", // must >= SBI version 0.3
        "j      2f",
        "1:",
        "la     sp, {boot_stack}",
        "li     t2, {boot_stack_size}",
        "add    sp, sp, t2",
        "mv     a0, a1",
        "tail   {err_sbi_version}",
        "unimp",
        "2:",
        "la     sp, {boot_stack}",
        "li     t2, {boot_stack_size}",
        "add    sp, sp, t2",
        "mv     a0, t0",
        "mv     a1, t1",
        "tail   {rust_init}",
        "unimp", // unreachable
        boot_stack = sym BOOT_STACK,
        boot_stack_size = const BOOT_STACK_SIZE,
        rust_init = sym rust_init,
        err_sbi_version = sym err_sbi_version,
        options(noreturn)
    )
}

// fixme: better code format
unsafe extern "C" fn err_sbi_version(wrong_version: usize) -> ! {
    let string = "zihai: this hypervisor software must run over SBI version >= 0.3, but we have version ";
    for byte in string.bytes() {
        asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) byte as usize);
    }
    asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) b'0' as usize + (wrong_version >> 24));
    asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) b'.' as usize);
    asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) b'0' as usize + (wrong_version & 0xFF));
    asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) b'.' as usize);
    asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) b'\n' as usize);
    asm!("li a7, 0x08", "ecall"); // shutdown
    loop {}
}
