#![feature(asm_sym, asm_const, naked_functions)]
#![no_std]
#![no_main]

use core::arch::asm;
use core::mem::MaybeUninit;

// boot hart start
pub extern "C" fn rust_init(hartid: usize, opaque: usize) {
    // boot hart init
    unsafe {
        asm!("li a7, 0x01", "mv a6, {}", "ecall", in(reg) hartid + b'A');
    }
    // call sbi remote retentive suspension, use sbi 0.3 to wake other harts
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
        "beqz   a0, 2f",
        "1:",
        "wfi",
        "j      1b", // non-boot hart, halt
        "2:",
        "la     sp, {}",
        "li     t0, {}",
        "add    sp, sp, t0",
        "call   {}",
        "unimp", // unreachable
        sym BOOT_STACK,
        const BOOT_STACK_SIZE,
        sym rust_init,
        options(noreturn)
    )
}
