// Detect instruction sets (ISA extensions) by trap-and-return procedure

// First, we disable all S-level interrupts. Remaining traps in RISC-V core
// are all exceptions. Then, we filter out illegal instruction from exceptions.

// use core::arch::riscv64;
use core::arch::asm;
use riscv::register::{sstatus, stvec::{self, Stvec, TrapMode}, scause::{Scause, Trap, Exception}};

// detect if hypervisor extension exists on current hart environment
//
// this function tries to read hgatp and returns false if the read operation failed.
pub fn detect_h_extension() -> bool {
    // disable interrupts and handle exceptions only
    unsafe { sstatus::clear_sie() };
    let stvec = unsafe { init_detect_trap() };
    let tp: usize;
    unsafe { asm!("mv   {}, tp", out(reg) tp) };
    // if exception occurred, set answer to failed
    unsafe { asm!("li   tp, 1") }; // 1 => success, 0 => failed
    // try to read hgatp.
    unsafe {
        asm!("csrr  {}, 0x680", out(reg) _); // 0x680 => hgatp
    }
    let ans: usize;
    unsafe { asm!("mv   {}, tp", out(reg) ans) };
    let ans = ans != 0;
    // restore trap handler and enable interrupts
    unsafe { asm!("mv   tp, {}", in(reg) tp) };
    unsafe { restore_detect_trap(stvec) };
    unsafe { sstatus::set_sie() };
    // return the answer
    ans
}

extern "C" fn rust_detect_trap(trap_frame: &mut TrapFrame) {
    match trap_frame.scause.cause() {
        Trap::Exception(Exception::IllegalInstruction) => {
            trap_frame.tp = 0; // failed
        },
        Trap::Interrupt(_) | Trap::Exception(_) => unreachable!(),
    }
    trap_frame.sepc = trap_frame.sepc.wrapping_add(4); // skip `csrr hgatp`
}

#[inline]
unsafe fn init_detect_trap() -> Stvec {
    let prev = stvec::read();
    let mut trap_addr = on_detect_trap as usize;
    if trap_addr & 0b1 != 0 {
        trap_addr += 0b1;
    }
    stvec::write(trap_addr, TrapMode::Direct);
    prev
}

#[inline]
unsafe fn restore_detect_trap(stvec: Stvec) {
    asm!("csrw  stvec, {}", in(reg) stvec.bits())
}

#[repr(C)]
struct TrapFrame {
    a0: usize,
    a1: usize,
    a2: usize,
    a3: usize,
    a4: usize,
    a5: usize,
    a6: usize,
    a7: usize,
    t0: usize,
    t1: usize,
    t2: usize,
    t3: usize,
    t4: usize,
    t5: usize,
    t6: usize,
    tp: usize,
    sstatus: usize,
    sepc: usize,
    scause: Scause,
}

#[naked]
unsafe extern "C" fn on_detect_trap() -> ! {
    asm!(
        ".p2align 2",
        "addi   sp, sp, -8*19",
        "sd     a0, 0*8(sp)",
        "sd     a1, 1*8(sp)",
        "sd     a2, 2*8(sp)",
        "sd     a3, 3*8(sp)",
        "sd     a4, 4*8(sp)",
        "sd     a5, 5*8(sp)",
        "sd     a6, 6*8(sp)",
        "sd     a7, 7*8(sp)",
        "sd     t0, 8*8(sp)",
        "sd     t1, 9*8(sp)",
        "sd     t2, 10*8(sp)",
        "sd     t3, 11*8(sp)",
        "sd     t4, 12*8(sp)",
        "sd     t5, 13*8(sp)",
        "sd     t6, 14*8(sp)",
        "sd     tp, 15*8(sp)",
        "csrr   t0, sstatus",
        "sd     t0, 16*8(sp)",
        "csrr   t1, sepc",
        "sd     t1, 17*8(sp)",
        "csrr   t2, scause",
        "sd     t2, 18*8(sp)",
        "mv     a0, sp",
        "call   {rust_detect_trap}",
        "ld     t0, 16*8(sp)",
        "csrw   sstatus, t0",
        "ld     t1, 17*8(sp)",
        "csrw   sepc, t1",
        "ld     t2, 18*8(sp)",
        "csrw   scause, t2",
        "ld     a0, 0*8(sp)",
        "ld     a1, 1*8(sp)",
        "ld     a2, 2*8(sp)",
        "ld     a3, 3*8(sp)",
        "ld     a4, 4*8(sp)",
        "ld     a5, 5*8(sp)",
        "ld     a6, 6*8(sp)",
        "ld     a7, 7*8(sp)",
        "ld     t0, 8*8(sp)",
        "ld     t1, 9*8(sp)",
        "ld     t2, 10*8(sp)",
        "ld     t3, 11*8(sp)",
        "ld     t4, 12*8(sp)",
        "ld     t5, 13*8(sp)",
        "ld     t6, 14*8(sp)",
        "ld     tp, 15*8(sp)",
        "addi   sp, sp, 8*19",
        "sret",
        rust_detect_trap = sym rust_detect_trap,
        options(noreturn),
    )
}
