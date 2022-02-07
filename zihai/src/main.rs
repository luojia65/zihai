#![feature(asm_sym, asm_const, naked_functions, stdsimd, alloc_error_handler)]
#![no_std]
#![no_main]
extern crate alloc;

#[macro_use]
mod console;
mod mm;
mod sbi;

use core::arch::asm;
use core::mem::MaybeUninit;
use riscv::register::stvec::{self, TrapMode};

// boot hart start
pub extern "C" fn rust_init(hartid: usize, opaque: usize) {
    // boot hart init
    println!("Welcome to zihai hypervisor");
    let mut trap_addr = on_supervisor_trap as usize;
    if trap_addr & 0b1 != 0 {
        trap_addr += 0b1;
    }
    unsafe { stvec::write(trap_addr, TrapMode::Direct) };
    
    let hsm_version = sbi::probe_extension(0x48534D);
    if hsm_version == 0 { // HSM does not exist under current SBI environment
        panic!("no HSM extension exist under current SBI environment");
    }
    unsafe { asm!("unimp") }; // FIXME: detect extensions
    println!("zihai > init hart id: {}", hartid);
    println!("zihai > opaque register: {}", opaque);
    println!("zihai > SBI HSM probe identifier: {}", hsm_version);
    mm::heap_init();
    mm::test_frame_alloc();
    // there's only one frame allocator no matter how much core the system have
    let from = mm::PhysAddr(0x80420000).page_number::<mm::Sv39>();
    let to = mm::PhysAddr(0x80800000).page_number::<mm::Sv39>(); // fixed for qemu
    let frame_alloc = spin::Mutex::new(mm::StackFrameAllocator::new(from, to));
    let mut kernel_addr_space = mm::PagedAddrSpace::try_new_in(mm::Sv39, &frame_alloc)
        .expect("allocate page to create kernel paged address space");
    mm::test_map_solve();
    kernel_addr_space.allocate_map(
        mm::VirtAddr(0x80000000).page_number::<mm::Sv39>(), 
        mm::PhysAddr(0x80000000).page_number::<mm::Sv39>(), 
        1024,
        mm::Sv39Flags::R | mm::Sv39Flags::W | mm::Sv39Flags::X
    ).expect("allocate one mapped space");
    kernel_addr_space.allocate_map(
        mm::VirtAddr(0x80400000).page_number::<mm::Sv39>(), 
        mm::PhysAddr(0x80400000).page_number::<mm::Sv39>(), 
        32,
        mm::Sv39Flags::R | mm::Sv39Flags::W | mm::Sv39Flags::X
    ).expect("allocate user program mapped space");
    kernel_addr_space.allocate_map(
        mm::VirtAddr(0x80420000).page_number::<mm::Sv39>(), 
        mm::PhysAddr(0x80420000).page_number::<mm::Sv39>(), 
        1024 - 32, 
        mm::Sv39Flags::R | mm::Sv39Flags::W | mm::Sv39Flags::X
    ).expect("allocate remaining space");
    mm::test_asid_alloc();
    let max_asid = mm::max_asid();
    let mut asid_alloc = mm::StackAsidAllocator::new(max_asid);
    let kernel_asid = asid_alloc.allocate_asid().expect("alloc kernel asid");
    let _kernel_satp = unsafe {
        mm::activate_paged_riscv_sv39(kernel_addr_space.root_page_number(), kernel_asid)
    };
    println!("zihai > entered kernel virtual address space: {}", kernel_asid);

    // call sbi remote retentive suspension, use sbi 0.3 to wake other harts

    sbi::reset(0x00000000, 0x00000000); // shutdown // todo: remove
}

pub extern "C" fn rust_init_harts(_opaque: usize) {
    // join working queue, ...
}

// -- detect isa extensions in current hart --

#[naked]
unsafe extern "C" fn on_supervisor_trap() -> ! {
    asm!(
        ".p2align 2",
        "addi   sp, sp, -8*17",
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
        "csrr   t0, sstatus",
        "sd     t0, 15*8(sp)",
        "csrr   t1, sepc",
        "sd     t1, 16*8(sp)",
        "mv     a0, sp",
        "call   {rust_supervisor_trap}",
        "ld     t0, 15*8(sp)",
        "csrw   sstatus, t0",
        "ld     t1, 16*8(sp)",
        "csrw   sepc, t1",
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
        "addi   sp, sp, 8*17",
        "sret",
        rust_supervisor_trap = sym rust_supervisor_trap,
        options(noreturn),
    )
}

#[repr(C)]
struct TrapFrame {
    values: [usize; 17],
}

extern "C" fn rust_supervisor_trap(trap_frame: &mut TrapFrame) {
    println!("trapped"); // FIXME: detect extensions
    trap_frame.values[16] += 4; // sepc += 4
}

#[panic_handler]
fn on_panic(info: &core::panic::PanicInfo) -> ! {
    println!("{}", info);
    sbi::reset(0x00000000, 0x00000001)
}

const BOOT_STACK_SIZE: usize = 64 * 1024; // 64KB
static BOOT_STACK: MaybeUninit<[u8; BOOT_STACK_SIZE]> = MaybeUninit::uninit();

#[link_section = ".text.entry"]
#[export_name = "_start"]
#[naked]
pub unsafe extern "C" fn start() -> ! {
    asm!(
        // prepare stack
        "la     sp, {boot_stack}",
        "li     t2, {boot_stack_size}",
        "addi   t3, a0, 1",
        "mul    t2, t2, t3",
        "add    sp, sp, t2",
        // start boot hart
        "beqz   a0, 2f",
        "mv     t0, a0",
        "mv     t1, a1",
        // stop other harts
        "li     a7, 0x48534D",
        "li     a6, 0x3", // hart suspend
        "li     a0, 0x80000000",  // suspend type: non retentive
        "la     a1, {rust_init_harts}", // resume address
        "mv     a2, t1", // a2: opaque parameter
        "ecall", // SBI hart syspend
        "1:",
        "wfi", // suspend failed, use WFI-loop halt instead
        "j      1b", // non-boot hart, halt
        "2:",
        // detect SBI version
        "li     a7, 0x10", // function id
        "li     a6, 0x0", // get spec version
        "ecall", // call SBI environment
        "li     t2, 0x00000003",
        "blt    a1, t2, 1f", // must >= SBI version 0.3
        "j      2f",
        "1:",
        "mv     a0, a1",
        "tail   {err_sbi_version}",
        "unimp",
        "2:",
        "mv     a0, t0",
        "mv     a1, t1",
        "tail   {rust_init}",
        "unimp", // unreachable
        boot_stack = sym BOOT_STACK,
        boot_stack_size = const BOOT_STACK_SIZE,
        rust_init = sym rust_init,
        rust_init_harts = sym rust_init_harts,
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
