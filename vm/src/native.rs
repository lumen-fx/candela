//! Running the machine code a `.cdlb` carries.
//!
//! `candela build` compiles the functions it can to machine code and puts
//! the code in the artifact beside the bytecode, one section per kind of
//! machine ([`NativeImage`]). Loading a program picks the section this
//! machine can run, copies its code into memory of its own, makes that memory
//! executable (never writable and executable at once), and rewrites the calls
//! the section covers so they call the machine code. Everything else runs on
//! the interpreter, and any section that does not fit this machine, or memory
//! the operating system refuses to make executable, leaves the whole program
//! on bytecode, which is complete on its own.
//!
//! The code reaches the VM only through a [`NativeCtx`]: the register file,
//! the object pool, a few status words and the helpers it calls for anything
//! the VM does itself. The contract between the two is versioned by
//! [`abi::VERSION`].

use crate::artifact::NativeArch;
use crate::artifact::NativeImage;
use crate::artifact::NativeOs;
use crate::data::Data;
use crate::instr::Instr;

/// The contract between generated code and the VM: the layout of the
/// context, the status codes, and the layout of an object in the pool.
pub mod abi {
    use super::NativeCtx;
    use core::mem::offset_of;

    /// Bumped whenever anything below, the helpers' signatures, or the layout
    /// of a value changes.
    pub const VERSION: u32 = 1;

    pub const CTX_REGS: i32 = offset_of!(NativeCtx, regs) as i32;
    pub const CTX_OBJS: i32 = offset_of!(NativeCtx, objs) as i32;
    pub const CTX_STATUS: i32 = offset_of!(NativeCtx, status) as i32;
    pub const CTX_ERR_AT: i32 = offset_of!(NativeCtx, err_at) as i32;
    pub const CTX_ERR_A: i32 = offset_of!(NativeCtx, err_a) as i32;
    pub const CTX_ERR_B: i32 = offset_of!(NativeCtx, err_b) as i32;
    pub const CTX_MARKING: i32 = offset_of!(NativeCtx, marking) as i32;
    pub const CTX_RET: i32 = offset_of!(NativeCtx, ret) as i32;
    pub const CTX_DYLIBS: i32 = offset_of!(NativeCtx, dylibs) as i32;
    pub const CTX_SHADE: i32 = offset_of!(NativeCtx, shade) as i32;
    pub const CTX_PRINT: i32 = offset_of!(NativeCtx, print) as i32;
    pub const CTX_CALL_VM: i32 = offset_of!(NativeCtx, call_vm) as i32;
    pub const CTX_PUSH_ROOTS: i32 = offset_of!(NativeCtx, push_roots) as i32;
    pub const CTX_POP_ROOTS: i32 = offset_of!(NativeCtx, pop_roots) as i32;

    /// The call went through.
    pub const STATUS_OK: u32 = 0;
    /// A list index was out of range: `err_a` is the length, `err_b` the index.
    pub const STATUS_INDEX: u32 = 1;
    /// A call would have stood past the call depth limit.
    pub const STATUS_DEPTH: u32 = 2;
    /// `main` ran to its end.
    pub const STATUS_HALT: u32 = 3;
    /// The interpreter, entered from the code, stopped with an error it
    /// recorded itself.
    pub const STATUS_VM: u32 = 4;
    /// A shift count was outside 0..64: `err_a` is the count.
    pub const STATUS_SHIFT: u32 = 5;
    /// A panic in a host function is on its way out; the VM holds it.
    pub const STATUS_PANIC: u32 = 6;

    /// How an object in the pool is laid out: the size of one entry, and the
    /// offsets of its buffer pointer and its length. The pool is a vector of
    /// vectors the code indexes directly, and Rust promises no layout for a
    /// vector, so the build measures it and the VM checks the build's answer
    /// against its own before it runs any of the code.
    #[must_use]
    pub fn object_layout() -> [u8; 3] {
        let mut probe: Vec<crate::data::Data> = Vec::with_capacity(7);
        probe.push(crate::data::NULL);
        probe.push(crate::data::NULL);
        probe.push(crate::data::NULL);
        let words: [usize; 3] = unsafe { core::mem::transmute_copy(&probe) };
        let offset = |value: usize| {
            words
                .iter()
                .position(|word| *word == value)
                .map_or(u8::MAX, |i| (i * size_of::<usize>()) as u8)
        };
        [
            size_of::<Vec<crate::data::Data>>() as u8,
            offset(probe.as_ptr() as usize),
            offset(probe.len()),
        ]
    }
}

/// What a native function and the VM share while it runs. One lives for each
/// run of the interpreter, and every call into native code during that run
/// reuses it.
#[repr(C)]
pub struct NativeCtx {
    /// The register file.
    pub regs: *mut Data,
    /// The object pool's first entry. The pool can move when it grows, so
    /// the VM refreshes this whenever control comes back from it.
    pub objs: *const u8,
    /// Why the code stopped, one of the `abi::STATUS_*` codes.
    pub status: u32,
    /// The instruction a failure happened at.
    pub err_at: u32,
    pub err_a: i64,
    pub err_b: i64,
    /// Whether the collector is marking, when a store has to shade what it
    /// overwrites.
    pub marking: u64,
    /// The two words of the value an entry point returns.
    pub ret: [u64; 2],
    /// The call depth native code may recurse to before a call carries on in
    /// the interpreter; each native function is handed how far below it it
    /// stands.
    pub ceiling: i64,
    /// The call depth below the first function of the native code running.
    pub segment_base: i64,
    /// The resolved address of each `dylib` function, in binding order.
    pub dylibs: *const *const u8,
    /// The interpreter the helpers work on.
    pub machine: *mut core::ffi::c_void,
    /// Shades a value a store overwrites while the collector marks.
    pub shade: extern "C" fn(*mut Self, u64, u64),
    /// Prints a value.
    pub print: extern "C" fn(*mut Self, u64, u64),
    /// Runs a function on the interpreter: its first instruction, the
    /// register its result lands in, the call instruction, how far below the
    /// ceiling the caller stands, and the values to keep alive meanwhile.
    pub call_vm: extern "C" fn(*mut Self, u64, u64, u64, i64, *const u64, u64) -> u32,
    /// Keeps values the code holds alive across a call that can collect.
    pub push_roots: extern "C" fn(*mut Self, *const u64, u64),
    /// Lets go of values [`Self::push_roots`] kept.
    pub pop_roots: extern "C" fn(*mut Self, u64),
}

/// The signature of every entry point: the context, and how far below the
/// ceiling the function stands.
pub type Entry = unsafe extern "C" fn(*mut NativeCtx, i64) -> u32;

/// One function of the loaded code.
pub struct NativeFunction {
    pub entry: Entry,
    /// The instruction its bytecode starts at.
    pub loc: u32,
    pub returns: bool,
}

/// The machine code of a loaded program.
pub struct NativeCode {
    _memory: CodeMemory,
    pub functions: Vec<NativeFunction>,
    /// The function that stands in for `main`.
    pub main: Option<usize>,
    /// The calls the load rewrote, with the instruction each was, in
    /// instruction order. Errors name the instruction a program was compiled
    /// with.
    originals: Vec<(u32, Instr)>,
    /// Where a run the code starts on the interpreter returns to.
    pub exit: u32,
    /// The most stack one native call takes.
    pub frame_bytes: u32,
    /// The resolved `dylib` functions, for the code to call.
    pub dylibs: Vec<*const u8>,
}

impl NativeCode {
    /// The instruction the program was compiled with at `at`, which is
    /// `current` unless the load rewrote it.
    #[must_use]
    pub fn original(&self, at: usize, current: Instr) -> Instr {
        if !matches!(current, Instr::CallNative(..) | Instr::CallNativeRec(..)) {
            return current;
        }
        self.originals
            .binary_search_by_key(&(at as u32), |(i, _)| *i)
            .map_or(current, |k| self.originals[k].1)
    }
}

/// This machine, as a section names what it was built for.
const HOST: Option<(NativeArch, NativeOs)> = {
    let arch = if cfg!(target_arch = "x86_64") {
        Some(NativeArch::X86_64)
    } else if cfg!(target_arch = "aarch64") {
        Some(NativeArch::Aarch64)
    } else {
        None
    };
    let os = if cfg!(target_os = "linux") {
        Some(NativeOs::Linux)
    } else if cfg!(target_os = "macos") {
        Some(NativeOs::MacOs)
    } else if cfg!(target_os = "windows") {
        Some(NativeOs::Windows)
    } else {
        None
    };
    match (arch, os) {
        (Some(arch), Some(os)) => Some((arch, os)),
        _ => None,
    }
};

/// Whether this machine has the instruction set extension `feature`, by the
/// name Rust's feature detection uses. A name it does not know is a feature
/// it does not have.
fn cpu_has(feature: &str) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        match feature {
            "sse3" => std::arch::is_x86_feature_detected!("sse3"),
            "ssse3" => std::arch::is_x86_feature_detected!("ssse3"),
            "sse4.1" => std::arch::is_x86_feature_detected!("sse4.1"),
            "sse4.2" => std::arch::is_x86_feature_detected!("sse4.2"),
            "popcnt" => std::arch::is_x86_feature_detected!("popcnt"),
            "avx" => std::arch::is_x86_feature_detected!("avx"),
            "avx2" => std::arch::is_x86_feature_detected!("avx2"),
            "bmi1" => std::arch::is_x86_feature_detected!("bmi1"),
            "bmi2" => std::arch::is_x86_feature_detected!("bmi2"),
            "lzcnt" => std::arch::is_x86_feature_detected!("lzcnt"),
            "fma" => std::arch::is_x86_feature_detected!("fma"),
            _ => false,
        }
    }
    #[cfg(all(target_arch = "aarch64", not(target_os = "windows")))]
    {
        match feature {
            "lse" => std::arch::is_aarch64_feature_detected!("lse"),
            "fp16" => std::arch::is_aarch64_feature_detected!("fp16"),
            "paca" => std::arch::is_aarch64_feature_detected!("paca"),
            _ => false,
        }
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        all(target_arch = "aarch64", not(target_os = "windows"))
    )))]
    {
        let _ = feature;
        false
    }
}

/// Whether this VM can run `section`: built for this architecture and
/// operating system, against this contract and object layout, for a
/// processor this one is at least.
fn fits(section: &NativeImage) -> bool {
    HOST == Some((section.arch, section.os))
        && section.abi == abi::VERSION
        && section.object_layout == abi::object_layout()
        && section.cpu_features.iter().all(|f| cpu_has(f))
}

/// Whether `section` describes code that lines up with `instructions`: every
/// entry inside the code, every rewritten call a call of the function it
/// names. A section that fails this was damaged or built for another
/// program, and runs nothing.
fn consistent(section: &NativeImage, instructions: &[Instr]) -> bool {
    let functions = &section.functions;
    // The run a call into the interpreter starts returns to one instruction
    // past the end, and a return address is 16 bits.
    instructions.len() <= usize::from(u16::MAX)
        && !functions.is_empty()
        && functions.len() <= usize::from(u16::MAX)
        && section.frame_bytes > 0
        && functions.iter().all(|f| {
            (f.offset as usize) < section.code.len() && (f.loc as usize) < instructions.len()
        })
        && section
            .main
            .is_none_or(|main| (main as usize) < functions.len())
        && section.sites.iter().all(|site| {
            functions.get(site.function as usize).is_some_and(|f| {
                matches!(
                    instructions.get(site.at as usize),
                    Some(Instr::CallFunc(loc, _) | Instr::CallFuncRecursive(loc, _))
                        if u32::from(*loc) == f.loc
                )
            })
        })
}

/// Loads the first of `sections` this machine can run: maps its code, and
/// rewrites the calls it covers in `instructions`. Answers `None`, leaving
/// `instructions` as they were, when no section fits or the code cannot be
/// made executable; the program then runs on bytecode alone.
pub fn load(
    sections: &[NativeImage],
    instructions: &mut Vec<Instr>,
    dylibs: Vec<*const u8>,
) -> Option<NativeCode> {
    let section = sections.iter().find(|s| fits(s))?;
    if !consistent(section, instructions) {
        return None;
    }
    let memory = CodeMemory::new(&section.code)?;

    let functions = section
        .functions
        .iter()
        .map(|f| NativeFunction {
            // SAFETY: `consistent` checked the offset lies inside the code,
            // and the build put an entry point there.
            entry: unsafe { std::mem::transmute::<*const u8, Entry>(memory.at(f.offset as usize)) },
            loc: f.loc,
            returns: f.returns,
        })
        .collect();

    let mut originals = Vec::with_capacity(section.sites.len());
    for site in &section.sites {
        let at = site.at as usize;
        let before = instructions[at];
        instructions[at] = match before {
            Instr::CallFunc(_, ret) => Instr::CallNative(site.function as u16, ret),
            Instr::CallFuncRecursive(_, ret) => Instr::CallNativeRec(site.function as u16, ret),
            other => other,
        };
        originals.push((site.at, before));
    }
    originals.sort_unstable_by_key(|(at, _)| *at);
    originals.dedup_by_key(|(at, _)| *at);

    let exit = instructions.len() as u32;
    instructions.push(Instr::NativeExit);

    Some(NativeCode {
        _memory: memory,
        functions,
        main: section.main.map(|m| m as usize),
        originals,
        exit,
        frame_bytes: section.frame_bytes,
        dylibs,
    })
}

/// Memory holding machine code, readable and executable and never writable
/// once the code is in it. Given back to the operating system on drop.
struct CodeMemory {
    ptr: *mut u8,
    len: usize,
}

impl CodeMemory {
    /// The address `offset` bytes into the code.
    fn at(&self, offset: usize) -> *const u8 {
        // SAFETY: the caller keeps `offset` inside the code.
        unsafe { self.ptr.add(offset) }
    }
}

#[cfg(unix)]
mod os {
    use core::ffi::c_int;
    use core::ffi::c_void;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: c_int,
            flags: c_int,
            fd: c_int,
            off: i64,
        ) -> *mut c_void;
        fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
        fn munmap(addr: *mut c_void, len: usize) -> c_int;
    }
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn sys_icache_invalidate(start: *mut c_void, len: usize);
    }

    const PROT_READ: c_int = 1;
    const PROT_WRITE: c_int = 2;
    const PROT_EXEC: c_int = 4;
    const MAP_PRIVATE: c_int = 2;
    #[cfg(target_os = "macos")]
    const MAP_ANONYMOUS: c_int = 0x1000;
    #[cfg(not(target_os = "macos"))]
    const MAP_ANONYMOUS: c_int = 0x20;

    /// Maps fresh memory, copies `code` in, and turns the memory from
    /// writable into executable. `None` when any step is refused.
    pub fn map_code(code: &[u8]) -> Option<*mut u8> {
        let len = code.len();
        unsafe {
            let ptr = mmap(
                core::ptr::null_mut(),
                len,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            );
            if ptr as isize == -1 || ptr.is_null() {
                return None;
            }
            core::ptr::copy_nonoverlapping(code.as_ptr(), ptr.cast::<u8>(), len);
            flush(ptr, len);
            if mprotect(ptr, len, PROT_READ | PROT_EXEC) != 0 {
                munmap(ptr, len);
                return None;
            }
            Some(ptr.cast())
        }
    }

    pub fn unmap(ptr: *mut u8, len: usize) {
        unsafe {
            munmap(ptr.cast(), len);
        }
    }

    /// Makes the instruction cache see freshly written code. Only needed
    /// where the caches are not kept coherent for code, which is aarch64.
    #[allow(unused_variables)]
    unsafe fn flush(ptr: *mut c_void, len: usize) {
        #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
        unsafe {
            sys_icache_invalidate(ptr, len);
        }
        #[cfg(all(target_arch = "aarch64", not(target_os = "macos")))]
        unsafe {
            super::flush_aarch64(ptr.cast(), len);
        }
    }
}

#[cfg(windows)]
mod os {
    use core::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(addr: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(addr: *mut c_void, size: usize, kind: u32) -> i32;
        fn FlushInstructionCache(process: *mut c_void, addr: *const c_void, size: usize) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;
    const PAGE_EXECUTE_READ: u32 = 0x20;

    /// Allocates fresh memory, copies `code` in, and turns the memory from
    /// writable into executable. `None` when any step is refused.
    pub fn map_code(code: &[u8]) -> Option<*mut u8> {
        let len = code.len();
        unsafe {
            let ptr = VirtualAlloc(
                core::ptr::null_mut(),
                len,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            );
            if ptr.is_null() {
                return None;
            }
            core::ptr::copy_nonoverlapping(code.as_ptr(), ptr.cast::<u8>(), len);
            let mut old = 0;
            if VirtualProtect(ptr, len, PAGE_EXECUTE_READ, &raw mut old) == 0 {
                VirtualFree(ptr, 0, MEM_RELEASE);
                return None;
            }
            FlushInstructionCache(GetCurrentProcess(), ptr, len);
            Some(ptr.cast())
        }
    }

    pub fn unmap(ptr: *mut u8, _len: usize) {
        unsafe {
            VirtualFree(ptr.cast(), 0, MEM_RELEASE);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod os {
    /// No executable memory on this target: programs run on bytecode.
    pub fn map_code(_code: &[u8]) -> Option<*mut u8> {
        None
    }

    pub fn unmap(_ptr: *mut u8, _len: usize) {}
}

/// Cleans the data cache and invalidates the instruction cache over freshly
/// written code, line by line, the way the architecture asks for before new
/// code runs.
#[cfg(all(target_arch = "aarch64", unix, not(target_os = "macos")))]
unsafe fn flush_aarch64(ptr: *const u8, len: usize) {
    let ctr: u64;
    unsafe {
        core::arch::asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack));
    }
    let dline = 4usize << ((ctr >> 16) & 0xf);
    let iline = 4usize << (ctr & 0xf);
    let start = ptr as usize;
    let end = start + len;
    let mut addr = start & !(dline - 1);
    while addr < end {
        unsafe {
            core::arch::asm!("dc cvau, {}", in(reg) addr, options(nostack));
        }
        addr += dline;
    }
    unsafe {
        core::arch::asm!("dsb ish", options(nostack));
    }
    let mut addr = start & !(iline - 1);
    while addr < end {
        unsafe {
            core::arch::asm!("ic ivau, {}", in(reg) addr, options(nostack));
        }
        addr += iline;
    }
    unsafe {
        core::arch::asm!("dsb ish", "isb", options(nostack));
    }
}

impl CodeMemory {
    fn new(code: &[u8]) -> Option<Self> {
        if code.is_empty() {
            return None;
        }
        let ptr = os::map_code(code)?;
        Some(Self {
            ptr,
            len: code.len(),
        })
    }
}

impl Drop for CodeMemory {
    fn drop(&mut self) {
        os::unmap(self.ptr, self.len);
    }
}
