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
    // run detection by trap on csrr instruction.
    let ans = with_detect_trap(0, || unsafe {
        asm!("csrr  {}, 0x680", out(reg) _, options(nomem, nostack)); // 0x680 => hgatp
    });
    // return the answer from output flag. 0 => success, 2 => failed, illegal instruction
    ans != 2
}

#[inline]
fn with_detect_trap(param: usize, f: impl FnOnce()) -> usize {
    // disable interrupts and handle exceptions only
    let (sie, stvec, tp) = unsafe { init_detect_trap(param) };
    // run detection inner
    f();
    // restore trap handler and enable interrupts
    let ans = unsafe { restore_detect_trap(sie, stvec, tp) };
    // return the answer
    ans
}

extern "C" fn rust_detect_trap(trap_frame: &mut TrapFrame) {
    // store returned exception id value into tp register
    // specially: illegal instruction => 2
    trap_frame.tp = trap_frame.scause.bits();
    // if illegal instruction, skip current instruction
    match trap_frame.scause.cause() {
        Trap::Exception(Exception::IllegalInstruction) => {
            let insn_bits = if trap_frame.stval != 0 {
                riscv_insn_bits(trap_frame.stval)
            } else {
                4 // FIXME: read instruction, then judge how bits it would read
            };
            // skip current instruction
            trap_frame.sepc = trap_frame.sepc.wrapping_add(insn_bits);
        },
        Trap::Exception(_) => unreachable!(), // FIXME: unexpected instruction errors
        Trap::Interrupt(_) => unreachable!(), // filtered out for sie == false
    }
}

#[inline]
fn riscv_insn_bits(insn: usize) -> usize {
    if insn & 0b11 != 0b11 {
        return 2; // 16-bit
    }
    if insn & 0b11100 != 0b11100 {
        return 4; // 32-bit
    }
    return 4 // unknown by now
}

// initialize environment for trap detection and filter in exception only
#[inline]
unsafe fn init_detect_trap(param: usize) -> (bool, Stvec, usize) {
    // clear SIE to handle exception only
    let stored_sie = sstatus::read().sie();
    sstatus::clear_sie();
    // use detect trap handler to handle exceptions
    let stored_stvec = stvec::read();
    let mut trap_addr = on_detect_trap as usize;
    if trap_addr & 0b1 != 0 {
        trap_addr += 0b1;
    }
    stvec::write(trap_addr, TrapMode::Direct);
    // store tp register. tp will be used to load parameter and store return value
    let stored_tp: usize;
    asm!("mv  {}, tp", "mv  tp, {}", out(reg) stored_tp, in(reg) param, options(nomem, nostack));
    // returns preserved previous hardware states
    (stored_sie, stored_stvec, stored_tp)
}

// restore previous hardware states before trap detection
#[inline]
unsafe fn restore_detect_trap(sie: bool, stvec: Stvec, tp: usize) -> usize {
    // read the return value from tp register, and restore tp value
    let ans: usize;
    asm!("mv  {}, tp", "mv  tp, {}", out(reg) ans, in(reg) tp, options(nomem, nostack));
    // restore trap vector settings
    asm!("csrw  stvec, {}", in(reg) stvec.bits(), options(nomem, nostack));
    // enable interrupts
    if sie {
        sstatus::set_sie();
    };
    ans
}

#[repr(C)]
struct TrapFrame {
    ra: usize,
    tp: usize,
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
    sstatus: usize,
    sepc: usize,
    scause: Scause,
    stval: usize,
}

#[naked]
unsafe extern "C" fn on_detect_trap() -> ! {
    asm!(
        ".p2align 2",
        "addi   sp, sp, -8*21",
        "sd     ra, 0*8(sp)",
        "sd     tp, 1*8(sp)",
        "sd     a0, 2*8(sp)",
        "sd     a1, 3*8(sp)",
        "sd     a2, 4*8(sp)",
        "sd     a3, 5*8(sp)",
        "sd     a4, 6*8(sp)",
        "sd     a5, 7*8(sp)",
        "sd     a6, 8*8(sp)",
        "sd     a7, 9*8(sp)",
        "sd     t0, 10*8(sp)",
        "sd     t1, 11*8(sp)",
        "sd     t2, 12*8(sp)",
        "sd     t3, 13*8(sp)",
        "sd     t4, 14*8(sp)",
        "sd     t5, 15*8(sp)",
        "sd     t6, 16*8(sp)",
        "csrr   t0, sstatus",
        "sd     t0, 17*8(sp)",
        "csrr   t1, sepc",
        "sd     t1, 18*8(sp)",
        "csrr   t2, scause",
        "sd     t2, 19*8(sp)",
        "csrr   t3, stval",
        "sd     t3, 20*8(sp)",
        "mv     a0, sp",
        "call   {rust_detect_trap}",
        "ld     t0, 17*8(sp)",
        "csrw   sstatus, t0",
        "ld     t1, 18*8(sp)",
        "csrw   sepc, t1",
        "ld     t2, 19*8(sp)",
        "csrw   scause, t2",
        "ld     t3, 20*8(sp)",
        "csrw   stval, t3",
        "ld     ra, 0*8(sp)",
        "ld     tp, 1*8(sp)",
        "ld     a0, 2*8(sp)",
        "ld     a1, 3*8(sp)",
        "ld     a2, 4*8(sp)",
        "ld     a3, 5*8(sp)",
        "ld     a4, 6*8(sp)",
        "ld     a5, 7*8(sp)",
        "ld     a6, 8*8(sp)",
        "ld     a7, 9*8(sp)",
        "ld     t0, 10*8(sp)",
        "ld     t1, 11*8(sp)",
        "ld     t2, 12*8(sp)",
        "ld     t3, 13*8(sp)",
        "ld     t4, 14*8(sp)",
        "ld     t5, 15*8(sp)",
        "ld     t6, 16*8(sp)",
        "addi   sp, sp, 8*21",
        "sret",
        rust_detect_trap = sym rust_detect_trap,
        options(noreturn),
    )
}
