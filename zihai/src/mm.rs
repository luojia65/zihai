//! Memory module
//!
//! Includes heap memory and virtual memory system
#![allow(unused)] // use in the future

use alloc::alloc::Layout;
use alloc::vec::Vec;
use core::arch::riscv64;
use core::{fmt, ops::Range};

use bit_field::BitField;
use buddy_system_allocator::LockedHeap;
use riscv::register::satp::{self, Mode, Satp};

const KERNEL_HEAP_SIZE: usize = 64 * 1024;

static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

#[global_allocator]
static HEAP: LockedHeap<32> = LockedHeap::empty();

#[cfg_attr(not(test), alloc_error_handler)]
#[allow(unused)]
fn alloc_error_handler(layout: Layout) -> ! {
    panic!("hypervisor alloc error for layout {:?}", layout)
}

pub(crate) fn heap_init() {
    unsafe {
        HEAP.lock()
            .init(HEAP_SPACE.as_ptr() as usize, KERNEL_HEAP_SIZE)
    }
    let mut vec = Vec::new();
    for i in 0..5 {
        vec.push(i);
    }
    if vec == [0, 1, 2, 3, 4] {
        println!("zihai > allocation test passed");
    } else {
        panic!("allocation test failed")
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PhysAddr(pub usize);

impl PhysAddr {
    pub fn page_number<M: PageMode>(&self) -> PhysPageNum {
        PhysPageNum(self.0 >> M::FRAME_SIZE_BITS)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct VirtAddr(pub usize);

impl VirtAddr {
    pub fn page_number<M: PageMode>(&self) -> VirtPageNum {
        VirtPageNum(self.0 >> M::FRAME_SIZE_BITS)
    }
    pub fn page_offset<M: PageMode>(&self, lvl: PageLevel) -> usize {
        self.0 & (M::get_layout_for_level(lvl).page_size::<M>() - 1)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PhysPageNum(usize);

impl PhysPageNum {
    pub fn addr_begin<M: PageMode>(&self) -> PhysAddr {
        PhysAddr(self.0 << M::FRAME_SIZE_BITS)
    }
    pub fn next_page(&self) -> PhysPageNum {
        // PhysPageNum不处理具体架构的PPN_BITS，它的合法性由具体架构保证
        PhysPageNum(self.0.wrapping_add(1))
    }
    pub fn is_within_range(&self, begin: PhysPageNum, end: PhysPageNum) -> bool {
        if begin.0 <= end.0 {
            begin.0 <= self.0 && self.0 < end.0
        } else {
            begin.0 <= self.0 || self.0 < end.0
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct VirtPageNum(usize);

impl VirtPageNum {
    // pub fn addr_begin<M: PageMode>(&self) -> VirtAddr {
    //     VirtAddr(self.0 << M::FRAME_SIZE_BITS)
    // }
    pub fn next_page_by_level<M: PageMode>(&self, lvl: PageLevel) -> VirtPageNum {
        let step = M::get_layout_for_level(lvl).align_in_frames();
        VirtPageNum(self.0.wrapping_add(step))
    }
}

// 页帧分配器。**对于物理空间的一个片段，只存在一个页帧分配器，无论有多少个处理核**
#[derive(Debug)]
pub struct StackFrameAllocator {
    current: PhysPageNum,
    end: PhysPageNum,
    recycled: Vec<PhysPageNum>,
}

impl StackFrameAllocator {
    pub fn new(start: PhysPageNum, end: PhysPageNum) -> Self {
        StackFrameAllocator {
            current: start,
            end,
            recycled: Vec::new(),
        }
    }
    pub fn allocate_frame(&mut self) -> Result<PhysPageNum, FrameAllocError> {
        if let Some(ppn) = self.recycled.pop() {
            Ok(ppn)
        } else {
            if self.current == self.end {
                Err(FrameAllocError)
            } else {
                let ans = self.current;
                self.current = self.current.next_page();
                Ok(ans)
            }
        }
    }
    pub fn deallocate_frame(&mut self, ppn: PhysPageNum) {
        // validity check
        if ppn.is_within_range(self.current, self.end)
            || self.recycled.iter().find(|&v| *v == ppn).is_some()
        {
            panic!("Frame ppn={:x?} has not been allocated!", ppn);
        }
        // recycle
        self.recycled.push(ppn);
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FrameAllocError;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct FrameLayoutError;

pub(crate) fn test_frame_alloc() {
    let from = PhysPageNum(0x80000);
    let to = PhysPageNum(0x100000);
    let mut alloc = StackFrameAllocator::new(from, to);
    let f1 = alloc.allocate_frame();
    assert_eq!(f1, Ok(PhysPageNum(0x80000)), "first allocation");
    let f2 = alloc.allocate_frame();
    assert_eq!(f2, Ok(PhysPageNum(0x80001)), "second allocation");
    alloc.deallocate_frame(f1.unwrap());
    let f3 = alloc.allocate_frame();
    assert_eq!(
        f3,
        Ok(PhysPageNum(0x80000)),
        "after free first, third allocation"
    );
    println!("zihai > frame allocator test passed");
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct AddressSpaceId(u16);

impl AddressSpaceId {
    fn next_asid(&self, max_asid: AddressSpaceId) -> Option<AddressSpaceId> {
        if self.0 >= max_asid.0 {
            None
        } else {
            Some(AddressSpaceId(self.0.wrapping_add(1)))
        }
    }
}

impl fmt::Display for AddressSpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

const DEFAULT_ASID: AddressSpaceId = AddressSpaceId(0); // RISC-V架构规定，必须实现

// 每个平台上是不一样的，需要通过读写satp寄存器获得
pub fn max_asid() -> AddressSpaceId {
    #[cfg(target_pointer_width = "64")]
    let mut val: usize = ((1 << 16) - 1) << 44;
    #[cfg(target_pointer_width = "32")]
    let mut val: usize = ((1 << 9) - 1) << 22;
    unsafe {
        core::arch::asm!("
        csrr    {tmp}, satp
        or      {val}, {tmp}, {val}
        csrw    satp, {val}
        csrrw   {val}, satp, {tmp}
    ", tmp = out(reg) _, val = inlateout(reg) val)
    };
    #[cfg(target_pointer_width = "64")]
    return AddressSpaceId(((val >> 44) & ((1 << 16) - 1)) as u16);
    #[cfg(target_pointer_width = "32")]
    return AddressSpaceId(((val >> 22) & ((1 << 9) - 1)) as u16);
}

// 在看代码的同志们可能发现，这里分配地址空间编号的算法和StackFrameAllocator很像。
// 这里需要注意的是，分配页帧的算法经常要被使用，而且包含很多参数，最好最快的写法不一定是简单的栈式回收分配，
// 更好的高性能内核设计，页帧分配的算法或许会有较大的优化空间。
// 可以包含的参数，比如，页帧的内存布局，包括内存对齐的选项，这是大页优化非常需要的选项。
// 但是地址空间编号的分配算法而且不需要经常调用，所以可以设计得很简单，普通栈式回收的算法就足够使用了。

// 地址空间编号分配器，**每个处理核都有一个**
#[derive(Debug)]
pub struct StackAsidAllocator {
    current: AddressSpaceId,
    exhausted: bool,
    max: AddressSpaceId,
    recycled: Vec<AddressSpaceId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct AsidAllocError;

impl StackAsidAllocator {
    pub fn new(max_asid: AddressSpaceId) -> Self {
        StackAsidAllocator {
            current: DEFAULT_ASID,
            exhausted: false,
            max: max_asid,
            recycled: Vec::new(),
        }
    }

    pub fn allocate_asid(&mut self) -> Result<AddressSpaceId, AsidAllocError> {
        if let Some(asid) = self.recycled.pop() {
            return Ok(asid);
        }
        if self.exhausted {
            return Err(AsidAllocError);
        }
        if self.current == self.max {
            self.exhausted = true;
            return Ok(self.max);
        }
        if let Some(next) = self.current.next_asid(self.max) {
            let ans = self.current;
            self.current = next;
            Ok(ans)
        } else {
            Err(AsidAllocError)
        }
    }

    fn deallocate_asid(&mut self, asid: AddressSpaceId) {
        if asid.next_asid(self.max).is_none()
            || self.recycled.iter().find(|&v| *v == asid).is_some()
        {
            panic!("Asid {:x?} has not been allocated!", asid);
        }
        self.recycled.push(asid);
    }
}

pub(crate) fn test_asid_alloc() {
    let max_asid = AddressSpaceId(0xffff);
    let mut alloc = StackAsidAllocator::new(max_asid);
    let a1 = alloc.allocate_asid();
    assert_eq!(a1, Ok(AddressSpaceId(0)), "first allocation");
    let a2 = alloc.allocate_asid();
    assert_eq!(a2, Ok(AddressSpaceId(1)), "second allocation");
    alloc.deallocate_asid(a1.unwrap());
    let a3 = alloc.allocate_asid();
    assert_eq!(
        a3,
        Ok(AddressSpaceId(0)),
        "after free first one, third allocation"
    );
    for _ in 0..max_asid.0 - 2 {
        alloc.allocate_asid().unwrap();
    }
    let an = alloc.allocate_asid();
    assert_eq!(an, Ok(max_asid), "last asid");
    let an = alloc.allocate_asid();
    assert_eq!(
        an,
        Err(AsidAllocError),
        "when asid exhausted, allocate next"
    );
    alloc.deallocate_asid(a2.unwrap());
    let an = alloc.allocate_asid();
    assert_eq!(
        an,
        Ok(AddressSpaceId(1)),
        "after free second one, allocate next"
    );
    let an = alloc.allocate_asid();
    assert_eq!(an, Err(AsidAllocError), "no asid remains, allocate next");

    let mut alloc = StackAsidAllocator::new(DEFAULT_ASID); // asid not implemented
    let a1 = alloc.allocate_asid();
    assert_eq!(
        a1,
        Ok(AddressSpaceId(0)),
        "asid not implemented, first allocation"
    );
    let a2 = alloc.allocate_asid();
    assert_eq!(
        a2,
        Err(AsidAllocError),
        "asid not implemented, second allocation"
    );

    println!("zihai > host address space allocator test passed");
}

pub trait FrameAllocator {
    fn allocate_frame(&self) -> Result<PhysPageNum, FrameAllocError>;
    fn deallocate_frame(&self, ppn: PhysPageNum);
}

pub type DefaultFrameAllocator = spin::Mutex<StackFrameAllocator>;

impl FrameAllocator for DefaultFrameAllocator {
    fn allocate_frame(&self) -> Result<PhysPageNum, FrameAllocError> {
        self.lock().allocate_frame()
    }
    fn deallocate_frame(&self, ppn: PhysPageNum) {
        self.lock().deallocate_frame(ppn)
    }
}

impl<A: FrameAllocator + ?Sized> FrameAllocator for &A {
    fn allocate_frame(&self) -> Result<PhysPageNum, FrameAllocError> {
        (**self).allocate_frame()
    }
    fn deallocate_frame(&self, ppn: PhysPageNum) {
        (**self).deallocate_frame(ppn)
    }
}

// 表示整个页帧内存的所有权
#[derive(Debug)]
pub struct FrameBox<A: FrameAllocator = DefaultFrameAllocator> {
    ppn: PhysPageNum, // 相当于*mut类型的指针
    frame_alloc: A,
}

impl<A: FrameAllocator> FrameBox<A> {
    // 分配页帧并创建FrameBox
    pub fn try_new_in(frame_alloc: A) -> Result<FrameBox<A>, FrameAllocError> {
        let ppn = frame_alloc.allocate_frame()?;
        Ok(FrameBox { ppn, frame_alloc })
    }
    // // unsafe说明。调用者必须保证以下约定：
    // // 1. ppn只被一个FrameBox拥有，也就是不能破坏所有权约定
    // // 2. 这个ppn是由frame_alloc分配的
    // unsafe fn from_ppn(ppn: PhysPageNum, frame_alloc: A) -> Self {
    //     Self { ppn, frame_alloc }
    // }

    // 得到本页帧内存的页号
    pub fn phys_page_num(&self) -> PhysPageNum {
        self.ppn
    }
}

impl<A: FrameAllocator> Drop for FrameBox<A> {
    fn drop(&mut self) {
        // 释放所占有的页帧
        self.frame_alloc.deallocate_frame(self.ppn);
    }
}

// 分页模式
//
// 在每个页式管理模式下，我们认为分页系统分为不同的等级，每一级如果存在大页页表，都应当有相应的对齐要求。
// 然后当前的页式管理模式，一定有一个固定的最大等级。
//
// 如果虚拟内存的模式是直接映射或者线性映射，这将不属于分页模式的范围。应当混合使用其它的地址空间，综合成为更大的地址空间。
pub trait PageMode: Copy {
    /// Number of binary bits of the number in bytes each *frame* would contain under current page mode.
    ///
    /// *Frames* are defined as memory blocks with the length of the greatest common divisor of all
    /// possible page table sizes under current page mode.
    /// Each page under current mode should contain integer multiples of frames.
    ///
    /// Simple page modes may contain only one frame for page table of any levels.
    /// If such mode contains only 4KiB sized pages, value of frame size bits would be 12.
    const FRAME_SIZE_BITS: usize;
    // 当前分页模式下，物理页号的位数
    const PPN_BITS: usize;
    /// Number of maximum page levels this mode supports
    const MAX_PAGE_LEVELS: u8;

    const PAGE_ENTRIES_BITS: u8;
    fn get_layout_for_level(level: PageLevel) -> PageLayout {
        // lowest possible leaf level alignment
        let mut align_in_frames = 1_usize;
        // for every higher level, physical address alignment would multiply by
        for _ in 0..level.0 {
            if align_in_frames.leading_zeros() < Self::PAGE_ENTRIES_BITS as u32 {
                panic!("too much page levels")
            }
            align_in_frames <<= Self::PAGE_ENTRIES_BITS;
        }
        unsafe { PageLayout::new_unchecked(align_in_frames) }
    }
    // 得到从高到低的页表等级
    fn visit_levels_until(level: PageLevel) -> LevelIter {
        assert!(level.0 < Self::MAX_PAGE_LEVELS, "page level doesn't exist");
        LevelIter::falling_includes(Self::MAX_PAGE_LEVELS - 1, level.0)
    }
    // 得到从高到低的页表等级，不包括level
    fn visit_levels_before(level: PageLevel) -> LevelIter {
        assert!(level.0 < Self::MAX_PAGE_LEVELS, "page level doesn't exist");
        LevelIter::falling_excludes(Self::MAX_PAGE_LEVELS - 1, level.0)
    }
    // 得到从高到低的页表等级
    fn visit_levels_from(level: PageLevel) -> LevelIter {
        assert!(level.0 < Self::MAX_PAGE_LEVELS, "page level doesn't exist");
        LevelIter::falling_includes(level.0, 0)
    }
    // 得到一个虚拟页号对应等级的索引
    fn vpn_index(vpn: VirtPageNum, level: PageLevel) -> usize;
    // 得到一段虚拟页号对应该等级索引的区间；如果超过此段最大的索引，返回索引的结束值为索引的最大值
    fn vpn_index_range(vpn_range: Range<VirtPageNum>, level: PageLevel) -> Range<usize>;
    // 得到虚拟页号在当前等级下重新索引得到的页号
    fn vpn_level_index(vpn: VirtPageNum, level: PageLevel, idx: usize) -> VirtPageNum;
    // 当前分页模式下，页表的类型
    type PageTable: core::ops::Index<usize, Output = Self::Slot> + core::ops::IndexMut<usize>;
    // 创建页表时，把它的所有条目设置为无效条目
    fn init_page_table(table: &mut Self::PageTable);
    // 页式管理模式，可能有效也可能无效的页表项类型
    type Slot;
    // 页式管理模式，有效的页表项类型
    type Entry;
    // 解释页表项目；如果项目无效，返回None，可以直接操作slot写入其它数据
    fn slot_try_get_entry(slot: &mut Self::Slot) -> Result<&mut Self::Entry, &mut Self::Slot>;
    // 页表项的设置
    type Flags: Clone;
    // 写数据，建立一个到子页表的页表项
    fn slot_set_child(slot: &mut Self::Slot, ppn: PhysPageNum);
    // 写数据，建立一个到内存地址的页表项
    fn slot_set_mapping(slot: &mut Self::Slot, ppn: PhysPageNum, flags: Self::Flags);
    // 判断页表项目是否是一个叶子节点
    fn entry_is_leaf_page(entry: &mut Self::Entry) -> bool;
    // 写数据到页表项目，说明这是一个叶子节点
    fn entry_write_ppn_flags(entry: &mut Self::Entry, ppn: PhysPageNum, flags: Self::Flags);
    // 得到一个页表项目包含的物理页号
    fn entry_get_ppn(entry: &Self::Entry) -> PhysPageNum;
}

/// Levels of paged memory systems
///
/// Higher page level of any page table tree would have bigger page level numbers,
/// the lowest possible leaf level would be defined as level zero.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PageLevel(u8);

impl PageLevel {
    /// Lowest leaf level possible of any paged memory system
    pub const fn leaf_level() -> Self {
        Self(0)
    }
}

/// Iterator of page levels, can be forward or backward.
#[derive(Clone, Eq, PartialEq)]
pub struct LevelIter {
    remaining_min: u8,
    remaining_max: u8,
    include_end: bool,
    towards_higher: bool,
}

impl LevelIter {
    #[allow(unused)]
    #[inline]
    fn rising_includes(begin: u8, end: u8) -> Self {
        assert!(begin <= end);
        LevelIter {
            remaining_min: begin,
            remaining_max: end,
            include_end: true,
            towards_higher: true,
        }
    }
    #[inline]
    fn falling_includes(begin: u8, end: u8) -> Self {
        assert!(begin >= end);
        LevelIter {
            remaining_min: end,
            remaining_max: begin,
            include_end: true,
            towards_higher: false,
        }
    }
    #[allow(unused)]
    #[inline]
    fn rising_excludes(begin: u8, end: u8) -> Self {
        assert!(begin <= end);
        LevelIter {
            remaining_min: begin,
            remaining_max: end,
            include_end: false,
            towards_higher: true,
        }
    }
    #[inline]
    fn falling_excludes(begin: u8, end: u8) -> Self {
        assert!(begin >= end);
        LevelIter {
            remaining_min: end,
            remaining_max: begin,
            include_end: false,
            towards_higher: false,
        }
    }
}

impl Iterator for LevelIter {
    type Item = PageLevel;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining_min == self.remaining_max {
            return if !self.include_end {
                None
            } else {
                self.include_end = false; // stop next iteration
                Some(PageLevel(self.remaining_min))
            };
        }
        let ans = if self.towards_higher {
            PageLevel(self.remaining_min)
        } else {
            PageLevel(self.remaining_max)
        };
        assert!(self.remaining_min < self.remaining_max);
        if self.towards_higher {
            self.remaining_min += 1;
        } else {
            self.remaining_max -= 1;
        }
        Some(ans)
    }
}

/// Size and alignment settings of pages
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PageLayout {
    /// How many *frames* this page would align to.
    ///
    /// If one frame equals to 4K bytes, then frame_align 1 describes alignment to
    /// 4K bytes, 512 describes 2M bytes, 512*512 describes 1G bytes.
    align_in_frames: usize,
}

// 应当从PageMode::get_layout_for_level中获得
impl PageLayout {
    // 未检查参数，用于实现PageMode
    pub const unsafe fn new_unchecked(align_in_frames: usize) -> Self {
        Self { align_in_frames }
    }
    pub const fn align_in_frames(&self) -> usize {
        self.align_in_frames
    }
    pub fn page_size<M: PageMode>(&self) -> usize {
        self.align_in_frames << M::FRAME_SIZE_BITS
    }
}

// Sv39分页系统模式；RISC-V RV64下有效
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Sv39;

impl PageMode for Sv39 {
    const FRAME_SIZE_BITS: usize = 12;
    const PPN_BITS: usize = 44;
    const MAX_PAGE_LEVELS: u8 = 3;
    const PAGE_ENTRIES_BITS: u8 = 9;
    fn vpn_index(vpn: VirtPageNum, level: PageLevel) -> usize {
        (vpn.0 >> (level.0 * 9)) & 511
    }
    fn vpn_index_range(vpn_range: Range<VirtPageNum>, level: PageLevel) -> Range<usize> {
        let start = (vpn_range.start.0 >> (level.0 * 9)) & 511;
        let mut end = (vpn_range.end.0 >> (level.0 * 9)) & 511;
        if level.0 <= 1 {
            let start_idx1 = vpn_range.start.0 >> ((level.0 + 1) * 9);
            let end_idx1 = vpn_range.end.0 >> ((level.0 + 1) * 9);
            if end_idx1 > start_idx1 {
                end = 512;
            }
        }
        start..end
    }
    fn vpn_level_index(vpn: VirtPageNum, level: PageLevel, idx: usize) -> VirtPageNum {
        VirtPageNum(match level.0 {
            0 => (vpn.0 & !((1 << 9) - 1)) + idx,
            1 => (vpn.0 & !((1 << 18) - 1)) + (idx << 9),
            2 => (vpn.0 & !((1 << 44) - 1)) + (idx << 18),
            _ => unimplemented!("this level does not exist on Sv39"),
        })
    }
    type PageTable = Sv39PageTable;
    fn init_page_table(table: &mut Self::PageTable) {
        // Zero init
        table.entries = unsafe { core::mem::MaybeUninit::zeroed().assume_init() };
    }
    type Slot = Sv39PageSlot;
    type Entry = Sv39PageEntry;
    fn slot_try_get_entry(
        slot: &mut Sv39PageSlot,
    ) -> Result<&mut Sv39PageEntry, &mut Sv39PageSlot> {
        // note(unsafe): slot是合法的
        let ans = unsafe { &mut *(slot as *mut _ as *mut Sv39PageEntry) };
        if ans.flags().contains(Sv39Flags::V) {
            Ok(ans)
        } else {
            Err(slot)
        }
    }
    type Flags = Sv39Flags;
    fn slot_set_child(slot: &mut Sv39PageSlot, ppn: PhysPageNum) {
        let ans = unsafe { &mut *(slot as *mut _ as *mut Sv39PageEntry) };
        ans.write_ppn_flags(ppn, Sv39Flags::V); // V=1, R=W=X=0
    }
    fn slot_set_mapping(slot: &mut Sv39PageSlot, ppn: PhysPageNum, flags: Sv39Flags) {
        let ans = unsafe { &mut *(slot as *mut _ as *mut Sv39PageEntry) };
        ans.write_ppn_flags(ppn, Sv39Flags::V | flags);
    }
    fn entry_is_leaf_page(entry: &mut Sv39PageEntry) -> bool {
        // 如果包含R、W或X项，就是叶子节点。
        entry
            .flags()
            .intersects(Sv39Flags::R | Sv39Flags::W | Sv39Flags::X)
    }
    fn entry_write_ppn_flags(entry: &mut Sv39PageEntry, ppn: PhysPageNum, flags: Sv39Flags) {
        entry.write_ppn_flags(ppn, flags);
    }
    fn entry_get_ppn(entry: &Sv39PageEntry) -> PhysPageNum {
        entry.ppn()
    }
}

#[repr(C)]
pub struct Sv39PageTable {
    entries: [Sv39PageSlot; 512],
}

impl core::ops::Index<usize> for Sv39PageTable {
    type Output = Sv39PageSlot;
    fn index(&self, idx: usize) -> &Sv39PageSlot {
        &self.entries[idx]
    }
}

impl core::ops::IndexMut<usize> for Sv39PageTable {
    fn index_mut(&mut self, idx: usize) -> &mut Sv39PageSlot {
        &mut self.entries[idx]
    }
}

#[repr(C)]
pub struct Sv39PageSlot {
    bits: usize,
}

#[repr(C)]
pub struct Sv39PageEntry {
    bits: usize,
}

impl Sv39PageEntry {
    #[inline]
    pub fn ppn(&self) -> PhysPageNum {
        PhysPageNum(self.bits.get_bits(10..54))
    }
    #[inline]
    pub fn flags(&self) -> Sv39Flags {
        Sv39Flags::from_bits_truncate(self.bits.get_bits(0..8) as u8)
    }
    #[inline]
    pub fn write_ppn_flags(&mut self, ppn: PhysPageNum, flags: Sv39Flags) {
        self.bits = (ppn.0 << 10) | flags.bits() as usize
    }
}

bitflags::bitflags! {
    pub struct Sv39Flags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

// Sv39x4 paged memory system; used in hypervisor G-stage address translation under RV64.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Sv39x4;

impl Sv39x4 {
    // returns vpn mask of Sv39x4 by page level
    #[inline]
    fn vpn_mask_by_level(level: PageLevel) -> usize {
        assert!(level.0 <= 2, "Sv39x4 only have page level 0, 1 or 2");
        // Vpn[2] would be 11 bits, Vpn[0..=1] would be 9 bits
        match level.0 {
            0..=1 => 511,
            2 => 2047,
            _ => unreachable!(),
        }
    }
}

// todo: To accommodate the 2 extra bits, the root page table (only)
// is expanded by a factor of four to be 16 KiB instead of the usual 4 KiB.
// Matching its larger size, the root page table also must be aligned to a 16 KiB
// boundary instead of the usual 4 KiB page boundary.

// Under Sv39x4, virtual address bits would be 41 other than 39;
// other attributes would be the same as Sv39.
// todo: incomplete design considering 16-KiB root page
impl PageMode for Sv39x4 {
    const FRAME_SIZE_BITS: usize = 12;
    const PPN_BITS: usize = 44;
    // Sv39x4 page levels are the same as Sv39 except that they are with bigger root pages
    const MAX_PAGE_LEVELS: u8 = 3;
    const PAGE_ENTRIES_BITS: u8 = 9;
    // In Sv39x4 vpn[2] would be 11 bits, vpn[0..=1] would be 9 bits
    fn vpn_index(vpn: VirtPageNum, level: PageLevel) -> usize {
        // `vpn_mask_by_level` will panic if `level` does not exist on Sv39x4
        (vpn.0 >> (level.0 * 9)) & Sv39x4::vpn_mask_by_level(level)
    }
    fn vpn_index_range(vpn_range: Range<VirtPageNum>, level: PageLevel) -> Range<usize> {
        let mask = Sv39x4::vpn_mask_by_level(level); // will panic if `level` does not <= 2
        let start = (vpn_range.start.0 >> (level.0 * 9)) & mask;
        let mut end = (vpn_range.end.0 >> (level.0 * 9)) & mask;
        if level.0 <= 1 {
            let start_idx1 = vpn_range.start.0 >> ((level.0 + 1) * 9);
            let end_idx1 = vpn_range.end.0 >> ((level.0 + 1) * 9);
            if end_idx1 > start_idx1 {
                end = mask + 1;
            }
        }
        start..end
    }
    fn vpn_level_index(vpn: VirtPageNum, level: PageLevel, idx: usize) -> VirtPageNum {
        Sv39::vpn_level_index(vpn, level, idx) // todo: figure out what is this
    }
    // Other than root table being 16-KiB, Sv39x4 has the same page table design as Sv39
    type PageTable = Sv39PageTable;
    // todo: 16-KiB root page table
    fn init_page_table(table: &mut Self::PageTable) {
        Sv39::init_page_table(table)
    }
    // Sv39x4 has same page table entry structure as Sv39
    type Slot = Sv39PageSlot;
    type Entry = Sv39PageEntry;
    fn slot_try_get_entry(slot: &mut Self::Slot) -> Result<&mut Self::Entry, &mut Self::Slot> {
        Sv39::slot_try_get_entry(slot)
    }
    type Flags = Sv39Flags;
    fn slot_set_child(slot: &mut Self::Slot, ppn: PhysPageNum) {
        Sv39::slot_set_child(slot, ppn)
    }
    fn slot_set_mapping(slot: &mut Self::Slot, ppn: PhysPageNum, flags: Self::Flags) {
        Sv39::slot_set_mapping(slot, ppn, flags)
    }
    fn entry_is_leaf_page(entry: &mut Self::Entry) -> bool {
        Sv39::entry_is_leaf_page(entry)
    }
    fn entry_write_ppn_flags(entry: &mut Self::Entry, ppn: PhysPageNum, flags: Self::Flags) {
        Sv39::entry_write_ppn_flags(entry, ppn, flags)
    }
    fn entry_get_ppn(entry: &Self::Entry) -> PhysPageNum {
        Sv39::entry_get_ppn(entry)
    }
}

// 表示一个分页系统实现的地址空间
//
// 如果属于直接映射或者线性偏移映射，不应当使用这个结构体，应当使用其它的结构体。
#[derive(Debug)]
pub struct PagedAddrSpace<M: PageMode, A: FrameAllocator = DefaultFrameAllocator> {
    root_frame: FrameBox<A>,
    frames: Vec<FrameBox<A>>,
    frame_alloc: A,
    page_mode: M,
}

impl<M: PageMode, A: FrameAllocator + Clone> PagedAddrSpace<M, A> {
    // 创建一个空的分页地址空间。一定会产生内存的写操作
    pub fn try_new_in(page_mode: M, frame_alloc: A) -> Result<Self, FrameAllocError> {
        // 新建一个满足根页表对齐要求的帧；虽然代码没有体现，通常对齐要求是1
        let mut root_frame = FrameBox::try_new_in(frame_alloc.clone())?;
        // println!("[kernel-alloc-map-test] Root frame: {:x?}", root_frame.phys_page_num());
        // 向帧里填入一个空的根页表
        unsafe { fill_frame_with_initialized_page_table::<A, M>(&mut root_frame) };
        Ok(Self {
            root_frame,
            frames: Vec::new(),
            frame_alloc,
            page_mode,
        })
    }
    // 得到根页表的地址
    pub fn root_page_number(&self) -> PhysPageNum {
        self.root_frame.phys_page_num()
    }
}

#[inline]
unsafe fn unref_ppn_mut<'a, M: PageMode>(ppn: PhysPageNum) -> &'a mut M::PageTable {
    let pa = ppn.addr_begin::<M>();
    &mut *(pa.0 as *mut M::PageTable)
}

// note: kernel identical mapping only
#[inline]
unsafe fn fill_frame_with_initialized_page_table<A: FrameAllocator, M: PageMode>(
    b: &mut FrameBox<A>,
) {
    let a = &mut *(b.ppn.addr_begin::<M>().0 as *mut M::PageTable);
    M::init_page_table(a);
}

impl<M: PageMode, A: FrameAllocator + Clone> PagedAddrSpace<M, A> {
    pub fn allocate_map(
        &mut self,
        vpn: VirtPageNum,
        ppn: PhysPageNum,
        n: usize,
        flags: M::Flags,
    ) -> Result<(), FrameAllocError> {
        for (page_level, vpn_range) in MapPairs::solve(vpn, ppn, n, self.page_mode) {
            // println!("[kernel-alloc-map-test] PAGE LEVEL: {:?}, VPN RANGE: {:x?}", page_level, vpn_range);
            let table = unsafe { self.alloc_get_table(page_level, vpn_range.start) }?;
            let idx_range = M::vpn_index_range(vpn_range.clone(), page_level);
            // println!("[kernel-alloc-map-test] IDX RANGE: {:?}", idx_range);
            for vidx in idx_range {
                let this_ppn = PhysPageNum(
                    ppn.0 + M::vpn_level_index(vpn_range.start, page_level, vidx).0 - vpn.0,
                );
                // println!("[kernel-alloc-map-test] Table: {:p} Vidx {} -> Ppn {:x?}", table, vidx, this_ppn);
                match M::slot_try_get_entry(&mut table[vidx]) {
                    Ok(_entry) => panic!("already allocated"),
                    Err(slot) => M::slot_set_mapping(slot, this_ppn, flags.clone()),
                }
            }
        }
        Ok(())
    }
}

impl<M: PageMode, A: FrameAllocator + Clone> PagedAddrSpace<M, A> {
    // 设置entry。如果寻找的过程中，中间的页表没创建，那么创建它们
    // should run on identical mapping (ppn == vpn) or paged mapping disabled
    unsafe fn alloc_get_table(
        &mut self,
        entry_level: PageLevel,
        vpn_start: VirtPageNum,
    ) -> Result<&mut M::PageTable, FrameAllocError> {
        let mut ppn = self.root_frame.phys_page_num();
        for level in M::visit_levels_before(entry_level) {
            // println!("[] BEFORE PPN = {:x?}", ppn);
            let page_table = unref_ppn_mut::<M>(ppn);
            let vidx = M::vpn_index(vpn_start, level);
            match M::slot_try_get_entry(&mut page_table[vidx]) {
                Ok(entry) => ppn = M::entry_get_ppn(entry),
                Err(mut slot) => {
                    // 需要一个内部页表，这里的页表项却没有数据，我们需要填写数据
                    let mut frame_box = FrameBox::try_new_in(self.frame_alloc.clone())?;
                    fill_frame_with_initialized_page_table::<A, M>(&mut frame_box);
                    M::slot_set_child(&mut slot, frame_box.phys_page_num());
                    // println!("[] Created a new frame box");
                    ppn = frame_box.phys_page_num();
                    self.frames.push(frame_box);
                }
            }
        }
        // println!("[kernel-alloc-map-test] in alloc_get_table PPN: {:x?}", ppn);
        let page_table = unref_ppn_mut::<M>(ppn); // 此时ppn是当前所需要修改的页表
                                                  // 创建了一个没有约束的生命周期。不过我们可以判断它是合法的，因为它的所有者是Self，在Self的周期内都合法
        Ok(&mut *(page_table as *mut _))
    }
    // pub fn unmap(&mut self, vpn: VirtPageNum) {
    //     todo!()
    // }

    /// 根据虚拟页号查询物理页号，可能出错。
    pub fn find_ppn(&self, vpn: VirtPageNum) -> Result<(&M::Entry, PageLevel), PageError> {
        let mut ppn = self.root_frame.phys_page_num();
        for lvl in M::visit_levels_until(PageLevel::leaf_level()) {
            // 注意: 要求内核对页表空间有恒等映射，可以直接解释物理地址
            let page_table = unsafe { unref_ppn_mut::<M>(ppn) };
            let vidx = M::vpn_index(vpn, lvl);
            match M::slot_try_get_entry(&mut page_table[vidx]) {
                Ok(entry) => {
                    if M::entry_is_leaf_page(entry) {
                        return Ok((entry, lvl));
                    } else {
                        ppn = M::entry_get_ppn(entry)
                    }
                }
                Err(_slot) => return Err(PageError::InvalidEntry),
            }
        }
        Err(PageError::NotLeafInLowestPage)
    }
}

/// 查询物理页号可能出现的错误
#[derive(Debug)]
pub enum PageError {
    /// 节点不具有有效位
    InvalidEntry,
    /// 第0层页表不能是内部节点
    NotLeafInLowestPage,
}

#[derive(Debug)]
pub struct MapPairs<M> {
    ans_iter: alloc::vec::IntoIter<(PageLevel, Range<VirtPageNum>)>,
    mode: M,
}

impl<M: PageMode> MapPairs<M> {
    pub fn solve(vpn: VirtPageNum, ppn: PhysPageNum, n: usize, mode: M) -> Self {
        let mut ans = Vec::new();
        for i in M::visit_levels_until(PageLevel::leaf_level()) {
            let align = M::get_layout_for_level(i).align_in_frames();
            if usize::wrapping_sub(vpn.0, ppn.0) % align != 0 || n < align {
                continue;
            }
            let (mut ve_prev, mut vs_prev) = (None, None);
            for j in M::visit_levels_from(i) {
                let align_cur = M::get_layout_for_level(j).align_in_frames();
                let ve_cur = align_cur * ((vpn.0 + align_cur - 1) / align_cur); // a * roundup(v / a)
                let vs_cur = align_cur * ((vpn.0 + n) / align_cur); // a * rounddown((v+n) / a)
                if let (Some(ve_prev), Some(vs_prev)) = (ve_prev, vs_prev) {
                    if ve_cur != ve_prev {
                        ans.push((j, VirtPageNum(ve_cur)..VirtPageNum(ve_prev)));
                    }
                    if vs_prev != vs_cur {
                        ans.push((j, VirtPageNum(vs_prev)..VirtPageNum(vs_cur)));
                    }
                } else {
                    if ve_cur != vs_cur {
                        ans.push((j, VirtPageNum(ve_cur)..VirtPageNum(vs_cur)));
                    }
                }
                (ve_prev, vs_prev) = (Some(ve_cur), Some(vs_cur));
            }
            break;
        }
        // println!("[SOLVE] Ans = {:x?}", ans);
        Self {
            ans_iter: ans.into_iter(),
            mode,
        }
    }
}

impl<M> Iterator for MapPairs<M> {
    type Item = (PageLevel, Range<VirtPageNum>);
    fn next(&mut self) -> Option<Self::Item> {
        self.ans_iter.next()
    }
}

pub(crate) fn test_map_solve() {
    let layout_frames_sv39 = [
        (PageLevel(0), 1),
        (PageLevel(1), 512),
        (PageLevel(2), 512 * 512),
    ];
    for (level, align_in_frames) in layout_frames_sv39 {
        assert_eq!(
            Sv39::get_layout_for_level(level).align_in_frames(),
            align_in_frames
        );
    }
    let visit_levels_until_sv39: [(PageLevel, &'static [PageLevel]); 3] = [
        (PageLevel(0), &[PageLevel(2), PageLevel(1), PageLevel(0)]),
        (PageLevel(1), &[PageLevel(2), PageLevel(1)]),
        (PageLevel(2), &[PageLevel(2)]),
    ];
    for (level, iter_result) in visit_levels_until_sv39 {
        assert_eq!(
            Sv39::visit_levels_until(level).collect::<Vec<_>>(),
            iter_result,
        );
    }
    let visit_levels_before_sv39: [(PageLevel, &'static [PageLevel]); 3] = [
        (PageLevel(0), &[PageLevel(2), PageLevel(1)]),
        (PageLevel(1), &[PageLevel(2)]),
        (PageLevel(2), &[]),
    ];
    for (level, iter_result) in visit_levels_before_sv39 {
        assert_eq!(
            Sv39::visit_levels_before(level).collect::<Vec<_>>(),
            iter_result,
        );
    }
    let visit_levels_from_sv39: [(PageLevel, &'static [PageLevel]); 3] = [
        (PageLevel(0), &[PageLevel(0)]),
        (PageLevel(1), &[PageLevel(1), PageLevel(0)]),
        (PageLevel(2), &[PageLevel(2), PageLevel(1), PageLevel(0)]),
    ];
    for (level, iter_result) in visit_levels_from_sv39 {
        assert_eq!(
            Sv39::visit_levels_from(level).collect::<Vec<_>>(),
            iter_result,
        );
    }

    let pairs = MapPairs::solve(VirtPageNum(0x90_000), PhysPageNum(0x50_000), 666666, Sv39)
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            (PageLevel(2), VirtPageNum(786432)..VirtPageNum(1048576)),
            (PageLevel(1), VirtPageNum(589824)..VirtPageNum(786432)),
            (PageLevel(1), VirtPageNum(1048576)..VirtPageNum(1256448)),
            (PageLevel(0), VirtPageNum(1256448)..VirtPageNum(1256490))
        ]
    );
    let pairs = MapPairs::solve(VirtPageNum(0x90_001), PhysPageNum(0x50_001), 77777, Sv39)
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            (PageLevel(1), VirtPageNum(590336)..VirtPageNum(667136)),
            (PageLevel(0), VirtPageNum(589825)..VirtPageNum(590336)),
            (PageLevel(0), VirtPageNum(667136)..VirtPageNum(667602))
        ]
    );
    let pairs = MapPairs::solve(VirtPageNum(0x12_345), PhysPageNum(0x22_345), 888888, Sv39x4)
        .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [
            (PageLevel(1), VirtPageNum(74752)..VirtPageNum(963072)),
            (PageLevel(0), VirtPageNum(74565)..VirtPageNum(74752)),
            (PageLevel(0), VirtPageNum(963072)..VirtPageNum(963453))
        ]
    );
    let pairs = MapPairs::solve(
        VirtPageNum(0x400000),
        PhysPageNum(0x200000),
        0x40000000,
        Sv39x4,
    )
    .collect::<Vec<_>>();
    assert_eq!(
        pairs,
        [(PageLevel(2), VirtPageNum(4194304)..VirtPageNum(1077936128))]
    );
    println!("zihai > address map solver test passed");
}

// activate Sv39 HS-mode supervisor translation
pub unsafe fn activate_supervisor_paged_riscv_sv39(
    root_ppn: PhysPageNum,
    asid: AddressSpaceId,
) -> Satp {
    satp::set(Mode::Sv39, asid.0 as usize, root_ppn.0);
    riscv64::sfence_vma_asid(asid.0 as usize);
    satp::read()
}

// 得到satp的值
pub fn get_satp_sv39(asid: AddressSpaceId, ppn: PhysPageNum) -> Satp {
    let bits = (8 << 60) | ((asid.0 as usize) << 44) | ppn.0;
    unsafe { core::mem::transmute(bits) }
}

// 帧翻译：在空间1中访问空间2的帧。要求空间1具有恒等映射特性
pub fn translate_frame_read</*M1, A1, */ M2, A2, F>(
    // as1: &PagedAddrSpace<M1, A1>,
    as2: &PagedAddrSpace<M2, A2>,
    vaddr2: VirtAddr,
    len_bytes2: usize,
    f: F,
) -> Result<(), PageError>
where
    // M1: PageMode,
    // A1: FrameAllocator + Clone,
    M2: PageMode,
    A2: FrameAllocator + Clone,
    F: Fn(PhysPageNum, usize, usize), // 按顺序返回空间1中的帧
{
    // println!("vaddr2 = {:x?}, len_bytes2 = {}", vaddr2, len_bytes2);
    let mut vpn2 = vaddr2.page_number::<M2>();
    let mut remaining_len = len_bytes2;
    let (mut entry, mut lvl) = as2.find_ppn(vpn2)?;
    let mut cur_offset = vaddr2.page_offset::<M2>(lvl);
    while remaining_len > 0 {
        let ppn = M2::entry_get_ppn(entry);
        let cur_frame_layout = M2::get_layout_for_level(lvl);
        let cur_len = if remaining_len <= cur_frame_layout.page_size::<M2>() {
            remaining_len
        } else {
            cur_frame_layout.page_size::<M2>()
        };
        f(ppn, cur_offset, cur_len);
        // println!("[] {} {} {}", cur_frame_layout.page_size::<M2>(), cur_offset, cur_len);
        remaining_len -= cur_len;
        if remaining_len == 0 {
            return Ok(());
        }
        cur_offset = 0; // 下一个帧从头开始
        vpn2 = vpn2.next_page_by_level::<M2>(lvl);
        (entry, lvl) = as2.find_ppn(vpn2)?;
        // println!("[] {}", remaining_len);
    }
    Ok(())
}
