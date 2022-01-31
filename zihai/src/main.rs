#![no_std]
#![no_main]

use core::arch::asm;
use core::mem::MaybeUninit;

// boot hart start
pub extern "C" fn rust_init() {
    // boot hart init

    // call sbi remote retentive suspension, use sbi 0.3 to wake other harts
}

pub extern "C" fn rust_init_harts() {
    // join working queue, ...
}

const BOOT_STACK_SIZE: usize = 64 * 1024; // 64KB
static BOOT_STACK: MaybeUninit<[u8; BOOT_STACK_SIZE]> = MaybeUninit::uninit();

#[export_name = "_start"]
pub unsafe extern "C" fn start() {
    asm!(
        "beqz   a0, 2f",
        "1:",
        "wfi",
        "j      1b", // non-boot hart, halt
        "2:",
        "li     sp, {}",
        "call   {}",
        "unimp", // unreachable
        in(reg) &BOOT_STACK,
        in(reg) rust_init,
    )
}
