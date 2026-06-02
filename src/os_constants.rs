//! # `os_enums_and_consts`
//!
//! Architecture- and environment-agnostic enumerations, constants, and plain
//! data types for use across Rust bare-metal and OS kernel implementations.
//!
//! ## Design contract
//!
//! - `#![no_std]` throughout. Nothing here pulls in `std`.
//! - Every type is `Copy + Clone + Debug + PartialEq + Eq + Hash` unless a
//!   field genuinely prevents it; the rationale is always noted inline.
//! - `#[non_exhaustive]` is applied to every enum that represents an open
//!   domain (ISAs, buses, firmware tables). Closed domains (privilege rings,
//!   endianness) are left exhaustive so `match` completeness is enforced.
//! - No `unsafe` in this file. Types are descriptors, not handles.
//! - Constants follow `SCREAMING_SNAKE_CASE`; types follow `UpperCamelCase`.
//!
//! ## Module layout
//!
//! ```text
//! os_enums_and_consts
//! ├── arch        — ISA, endianness, pointer width, register width
//! ├── boot        — bootloader protocols, firmware environments
//! ├── memory      — page sizes, memory region classifications, cache types
//! ├── privilege   — CPU privilege levels / rings / exception levels
//! ├── interrupts  — interrupt model, trigger modes, delivery modes
//! ├── bus         — peripheral bus types
//! ├── display     — pixel formats, display rotation
//! └── version     — crate version constant
//! ```

#![no_std]

// ═══════════════════════════════════════════════════════════════════════════════
// § arch — CPU Architecture
// ═══════════════════════════════════════════════════════════════════════════════

pub mod arch {
    /// CPU instruction set architecture.
    ///
    /// Covers every ISA with active Rust (`rustc`) target support or
    /// meaningful bare-metal relevance as of 2025. Fully deprecated ISAs
    /// (Alpha, PA-RISC, Itanium) are excluded.
    ///
    /// `#[non_exhaustive]` — new architectures (LoongArch, CHERI variants,
    /// future RISC-V profiles) will be added without breaking downstream
    /// exhaustive `match` arms.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum Architecture {
        // ── x86 family ────────────────────────────────────────────────────
        /// 32-bit x86 (IA-32 / i686). Legacy desktop and embedded.
        X86,
        /// 64-bit x86 with AMD64 extensions. Primary desktop/server ISA.
        X86_64,

        // ── ARM family ────────────────────────────────────────────────────
        /// 32-bit ARM, soft-float ABI (ARMv4T–ARMv7).
        Arm,
        /// 32-bit ARM, hard-float ABI (ARMv7-A with VFP/NEON).
        ArmHf,
        /// 64-bit ARM (AArch64 / ARMv8-A and later).
        Aarch64,

        // ── RISC-V ────────────────────────────────────────────────────────
        /// 32-bit RISC-V (RV32I base).
        RiscV32,
        /// 64-bit RISC-V (RV64I base). Primary target for serious OS work.
        RiscV64,

        // ── MIPS ──────────────────────────────────────────────────────────
        /// 32-bit MIPS, big-endian.
        Mips,
        /// 32-bit MIPS, little-endian.
        MipsEl,
        /// 64-bit MIPS, big-endian.
        Mips64,
        /// 64-bit MIPS, little-endian.
        Mips64El,

        // ── PowerPC ───────────────────────────────────────────────────────
        /// 32-bit PowerPC (embedded, legacy Apple).
        PowerPc,
        /// 64-bit PowerPC, big-endian (IBM POWER servers).
        PowerPc64,
        /// 64-bit PowerPC, little-endian / ELFv2 ABI (Linux on POWER).
        PowerPc64Le,

        // ── IBM z/Architecture ────────────────────────────────────────────
        /// IBM z/Architecture (s390x). Mainframe Linux.
        S390x,

        // ── Motorola 68k ──────────────────────────────────────────────────
        /// Motorola 68000 family. Retro/hobbyist and some embedded MCUs.
        M68k,

        // ── SPARC ─────────────────────────────────────────────────────────
        /// 64-bit SPARC (SPARCv9). Legacy Solaris/Linux servers.
        Sparc64,

        // ── LoongArch ─────────────────────────────────────────────────────
        /// 64-bit LoongArch. Upstream Linux support since 5.19; rustc target
        /// added in 1.71. Non-trivial geopolitical relevance going forward.
        LoongArch64,

        // ── WebAssembly ───────────────────────────────────────────────────
        /// WebAssembly 32-bit MVP. A first-class `rustc` target.
        Wasm32,
        /// WebAssembly 64-bit (memory64 proposal).
        Wasm64,
    }

    impl Architecture {
        /// Canonical `rustc` target triple architecture prefix.
        ///
        /// Concatenate with `-unknown-none` (or your preferred vendor/OS
        /// tuple) to form a complete bare-metal target triple.
        #[inline]
        pub const fn triple_prefix(self) -> &'static str {
            match self {
                Self::X86         => "i686",
                Self::X86_64      => "x86_64",
                Self::Arm         => "arm",
                Self::ArmHf       => "armv7",
                Self::Aarch64     => "aarch64",
                Self::RiscV32     => "riscv32",
                Self::RiscV64     => "riscv64",
                Self::Mips        => "mips",
                Self::MipsEl      => "mipsel",
                Self::Mips64      => "mips64",
                Self::Mips64El    => "mips64el",
                Self::PowerPc     => "powerpc",
                Self::PowerPc64   => "powerpc64",
                Self::PowerPc64Le => "powerpc64le",
                Self::S390x       => "s390x",
                Self::M68k        => "m68k",
                Self::Sparc64     => "sparc64",
                Self::LoongArch64 => "loongarch64",
                Self::Wasm32      => "wasm32",
                Self::Wasm64      => "wasm64",
            }
        }

        /// Native pointer width in bits.
        ///
        /// Returns `None` for `#[non_exhaustive]` variants added after this
        /// crate was compiled; callers should handle that case gracefully.
        #[inline]
        pub const fn pointer_width(self) -> u8 {
            match self {
                Self::X86
                | Self::Arm
                | Self::ArmHf
                | Self::RiscV32
                | Self::Mips
                | Self::MipsEl
                | Self::PowerPc
                | Self::M68k
                | Self::Wasm32 => 32,

                Self::X86_64
                | Self::Aarch64
                | Self::RiscV64
                | Self::Mips64
                | Self::Mips64El
                | Self::PowerPc64
                | Self::PowerPc64Le
                | Self::S390x
                | Self::Sparc64
                | Self::LoongArch64
                | Self::Wasm64 => 64,
            }
        }

        /// Returns `true` for ISAs where the toolchain produces distinct
        /// big-endian and little-endian targets.
        #[inline]
        pub const fn is_bi_endian(self) -> bool {
            matches!(
                self,
                Self::Arm
                    | Self::ArmHf
                    | Self::Mips
                    | Self::MipsEl
                    | Self::Mips64
                    | Self::Mips64El
                    | Self::PowerPc
                    | Self::PowerPc64
                    | Self::PowerPc64Le
                    | Self::Sparc64
            )
        }

        /// Native endianness for this specific variant.
        /// Bi-endian ISAs are represented by distinct enum arms, each with a
        /// fixed endianness, so this is always unambiguous.
        #[inline]
        pub const fn endianness(self) -> Endianness {
            match self {
                // Canonical big-endian variants
                Self::Mips
                | Self::Mips64
                | Self::PowerPc
                | Self::PowerPc64
                | Self::S390x
                | Self::Sparc64
                | Self::M68k => Endianness::Big,

                // Everything else in this enum is little-endian
                _ => Endianness::Little,
            }
        }
    }

    // ── Detect the compile-time host architecture ──────────────────────────────

    /// The architecture this code was compiled for, derived from `cfg` attributes.
    ///
    /// Returns `None` when the compiler target is not represented in
    /// [`Architecture`] (e.g. a future ISA not yet added here).
    #[inline]
    pub const fn current() -> Option<Architecture> {
        #[cfg(target_arch = "x86")]
        return Some(Architecture::X86);
        #[cfg(target_arch = "x86_64")]
        return Some(Architecture::X86_64);
        #[cfg(all(target_arch = "arm", target_feature = "vfp2"))]
        return Some(Architecture::ArmHf);
        #[cfg(all(target_arch = "arm", not(target_feature = "vfp2")))]
        return Some(Architecture::Arm);
        #[cfg(target_arch = "aarch64")]
        return Some(Architecture::Aarch64);
        #[cfg(all(target_arch = "riscv32"))]
        return Some(Architecture::RiscV32);
        #[cfg(all(target_arch = "riscv64"))]
        return Some(Architecture::RiscV64);
        #[cfg(all(target_arch = "mips", target_endian = "big"))]
        return Some(Architecture::Mips);
        #[cfg(all(target_arch = "mips", target_endian = "little"))]
        return Some(Architecture::MipsEl);
        #[cfg(all(target_arch = "mips64", target_endian = "big"))]
        return Some(Architecture::Mips64);
        #[cfg(all(target_arch = "mips64", target_endian = "little"))]
        return Some(Architecture::Mips64El);
        #[cfg(all(target_arch = "powerpc"))]
        return Some(Architecture::PowerPc);
        #[cfg(all(target_arch = "powerpc64", target_endian = "big"))]
        return Some(Architecture::PowerPc64);
        #[cfg(all(target_arch = "powerpc64", target_endian = "little"))]
        return Some(Architecture::PowerPc64Le);
        #[cfg(target_arch = "s390x")]
        return Some(Architecture::S390x);
        #[cfg(target_arch = "m68k")]
        return Some(Architecture::M68k);
        #[cfg(target_arch = "sparc64")]
        return Some(Architecture::Sparc64);
        #[cfg(target_arch = "loongarch64")]
        return Some(Architecture::LoongArch64);
        #[cfg(target_arch = "wasm32")]
        return Some(Architecture::Wasm32);
        #[cfg(target_arch = "wasm64")]
        return Some(Architecture::Wasm64);
        #[allow(unreachable_code)]
        None
    }

    // ── Endianness ────────────────────────────────────────────────────────────

    /// Byte order of a target or data structure.
    ///
    /// This enum is exhaustive — there are exactly two byte orders and no
    /// credible ISA is going to invent a third one.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Endianness {
        Little,
        Big,
    }

    impl Endianness {
        /// Endianness of the current compile target.
        #[inline]
        pub const fn native() -> Self {
            #[cfg(target_endian = "little")]
            return Self::Little;
            #[cfg(target_endian = "big")]
            return Self::Big;
        }

        #[inline]
        pub const fn is_little(self) -> bool { matches!(self, Self::Little) }

        #[inline]
        pub const fn is_big(self) -> bool { matches!(self, Self::Big) }
    }

    // ── Register width ────────────────────────────────────────────────────────

    /// General-purpose register width of a CPU.
    ///
    /// Distinct from pointer width: some ISAs (e.g. x32 ABI) use 64-bit
    /// registers with 32-bit pointers. For most bare-metal targets they
    /// are identical, but the distinction is captured here for correctness.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum RegisterWidth {
        Bits8,
        Bits16,
        Bits32,
        Bits64,
        Bits128,
    }

    impl RegisterWidth {
        /// Returns the width as a plain integer (in bits).
        #[inline]
        pub const fn bits(self) -> u16 {
            match self {
                Self::Bits8   => 8,
                Self::Bits16  => 16,
                Self::Bits32  => 32,
                Self::Bits64  => 64,
                Self::Bits128 => 128,
            }
        }

        /// Returns the width in bytes.
        #[inline]
        pub const fn bytes(self) -> u16 { self.bits() / 8 }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § boot — Bootloader Protocols and Firmware Environments
// ═══════════════════════════════════════════════════════════════════════════════

pub mod boot {
    /// Bootloader protocol the kernel was loaded by.
    ///
    /// Determines which boot structures are available at entry (e.g. which
    /// memory-map format to parse, which framebuffer query API to use).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum BootProtocol {
        /// Limine boot protocol (v1/v2). The modern x86_64/AArch64 choice.
        /// Provides HHDM, per-feature requests, and base revision negotiation.
        Limine,
        /// Multiboot 1 (GRUB legacy). 32-bit protected mode entry; no HHDM.
        Multiboot1,
        /// Multiboot 2 (GRUB2). Tag-based info structure; still 32-bit entry.
        Multiboot2,
        /// Raw EFI stub — the kernel is an EFI application and calls EFI
        /// boot services directly before exiting them.
        EfiStub,
        /// Linux boot protocol (bzImage format). Used by kernels that want
        /// compatibility with the Linux loader ecosystem.
        LinuxBoot,
        /// U-Boot / FIT image. Common on ARM embedded and SBCs.
        UBoot,
        /// No bootloader — kernel was loaded by the reset vector directly
        /// (typically embedded firmware or a stage-1 that jumps here raw).
        Bare,
        /// Unknown or not yet detected.
        Unknown,
    }

    /// Firmware / platform interface present at boot.
    ///
    /// In many cases both a bootloader protocol and a firmware type are
    /// relevant: a kernel might be loaded via Limine on top of UEFI, or
    /// via U-Boot on top of Device Tree.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum FirmwareKind {
        /// UEFI (Unified Extensible Firmware Interface). Provides runtime
        /// services, GOP framebuffer, and the ACPI/SMBIOS entry point.
        Uefi,
        /// Legacy BIOS (PC/AT). 16-bit real-mode origin; INT 15h memory map.
        Bios,
        /// Open Firmware / IEEE 1275. Found on SPARC, some PowerPC hardware.
        OpenFirmware,
        /// Flattened Device Tree (DTB). Ubiquitous on ARM/RISC-V embedded.
        DeviceTree,
        /// ACPI-only (no full UEFI). Some embedded x86 platforms.
        Acpi,
        /// No firmware interface — pure bare metal.
        None,
    }

    /// CPU execution state at kernel entry.
    ///
    /// Relevant for kernels that must verify or transition their initial
    /// execution environment before setting up their own GDT/page tables.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum EntryMode {
        /// x86: 16-bit real mode.
        RealMode,
        /// x86: 32-bit protected mode, paging optional.
        ProtectedMode,
        /// x86: 32-bit protected mode with paging enabled (Multiboot1/2).
        ProtectedModePaged,
        /// x86-64: 64-bit long mode. Interrupts and paging state
        /// depend on the bootloader; check [`BootProtocol`].
        LongMode,
        /// AArch64: EL1 (kernel privilege level). EL2 (hypervisor) is
        /// also possible on some boot paths.
        AArch64El1,
        /// AArch64: EL2 (hypervisor mode).
        AArch64El2,
        /// RISC-V: Supervisor mode (S-mode). M-mode firmware (OpenSBI) has
        /// already run and delegated.
        RiscVSupervisor,
        /// RISC-V: Machine mode (M-mode). No prior firmware abstraction layer.
        RiscVMachine,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § memory — Page Sizes, Region Types, Cache Attributes
// ═══════════════════════════════════════════════════════════════════════════════

pub mod memory {
    // ── Page size ─────────────────────────────────────────────────────────────

    /// Standard page sizes supported across common MMU implementations.
    ///
    /// Not all sizes are available on all architectures or in all page-table
    /// levels. Validate against your MMU's capabilities before use.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum PageSize {
        /// 4 KiB — the universal baseline. Every MMU with virtual memory
        /// supports this.
        Kib4,
        /// 16 KiB — native page size on Apple Silicon (AArch64) and some
        /// RISC-V configurations.
        Kib16,
        /// 64 KiB — ARM64 granule option; IBM POWER default.
        Kib64,
        /// 2 MiB — x86-64 huge page (PDT entry with PS bit); AArch64 large
        /// page with 4 KiB granule.
        Mib2,
        /// 1 GiB — x86-64 gigantic page (PDPT entry with PS bit). Requires
        /// PDPE1GB CPUID feature.
        Gib1,
    }

    impl PageSize {
        /// Page size in bytes.
        #[inline]
        pub const fn bytes(self) -> u64 {
            match self {
                Self::Kib4  =>        4 * 1024,
                Self::Kib16 =>       16 * 1024,
                Self::Kib64 =>       64 * 1024,
                Self::Mib2  =>  2 * 1024 * 1024,
                Self::Gib1  =>  1 * 1024 * 1024 * 1024,
            }
        }

        /// Log₂ of the page size. Useful for shift/mask operations.
        #[inline]
        pub const fn shift(self) -> u8 {
            match self {
                Self::Kib4  => 12,
                Self::Kib16 => 14,
                Self::Kib64 => 16,
                Self::Mib2  => 21,
                Self::Gib1  => 30,
            }
        }

        /// Alignment mask (`size - 1`). A physical address is aligned to this
        /// page size if `addr & mask == 0`.
        #[inline]
        pub const fn mask(self) -> u64 { self.bytes() - 1 }

        /// Returns `true` if `addr` is aligned to this page size.
        #[inline]
        pub const fn is_aligned(self, addr: u64) -> bool {
            addr & self.mask() == 0
        }

        /// Round `addr` down to the nearest boundary of this page size.
        #[inline]
        pub const fn align_down(self, addr: u64) -> u64 {
            addr & !self.mask()
        }

        /// Round `addr` up to the nearest boundary of this page size.
        ///
        /// Returns `None` on overflow (addr is within `mask` bytes of `u64::MAX`).
        #[inline]
        pub const fn align_up(self, addr: u64) -> Option<u64> {
            let mask = self.mask();
            addr.checked_add(mask).map(|a| a & !mask)
        }
    }

    // ── Physical memory region classification ─────────────────────────────────

    /// Classification of a physical memory region.
    ///
    /// Variants are ordered from most to least immediately exploitable by a
    /// buddy allocator. The ordering is intentional and should not be changed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    #[non_exhaustive]
    pub enum MemoryRegionKind {
        /// Freely available RAM. Hand to the physical page allocator immediately.
        Usable,
        /// RAM currently holding bootloader structures. Safe to reclaim after
        /// all bootloader data has been consumed into owned kernel state.
        BootloaderReclaimable,
        /// RAM holding ACPI tables. Reclaimable after the ACPI subsystem has
        /// copied what it needs.
        AcpiReclaimable,
        /// Non-volatile storage for ACPI S3/S4 resume. Do NOT reclaim.
        AcpiNvs,
        /// The kernel image and any loaded modules. Tracked separately so the
        /// VMM can map it as executable.
        KernelAndModules,
        /// Physical address range backing a firmware framebuffer.
        Framebuffer,
        /// Memory-mapped I/O region. No RAM; produce virtual mappings but
        /// never give to the frame allocator.
        Mmio,
        /// Persistent (non-volatile) memory (Intel Optane / NVDIMM).
        Persistent,
        /// Reported bad by firmware ECC scrubbing. Do not use.
        BadMemory,
        /// Firmware reserved. Do not touch.
        Reserved,
    }

    impl MemoryRegionKind {
        /// `true` if this region may be given to the frame allocator
        /// immediately at boot (before any reclaim phase).
        #[inline]
        pub const fn is_immediately_usable(self) -> bool {
            matches!(self, Self::Usable)
        }

        /// `true` if this region may be reclaimed into the frame allocator
        /// after the relevant subsystem (bootloader, ACPI) has been
        /// fully initialised and all data copied out.
        #[inline]
        pub const fn is_reclaimable(self) -> bool {
            matches!(self, Self::BootloaderReclaimable | Self::AcpiReclaimable)
        }

        /// `true` for regions that must never be given to the frame allocator.
        #[inline]
        pub const fn is_reserved(self) -> bool {
            !self.is_immediately_usable() && !self.is_reclaimable()
        }
    }

    // ── Cache / memory type ───────────────────────────────────────────────────

    /// CPU memory-type / cache attribute for a virtual or physical mapping.
    ///
    /// Vocabulary differs by ISA:
    /// - x86-64: MTRRs + PAT (Page Attribute Table) bits in PTEs
    /// - AArch64: MAIR_EL1 indices in page-table attribute fields
    /// - RISC-V: Svpbmt extension (`PBMT` field in PTEs)
    ///
    /// This enum uses the conceptual terms. Map to your ISA's concrete
    /// encoding in your MMU layer.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum CachePolicy {
        /// Fully cached, write-back. Normal RAM behaviour.
        WriteBack,
        /// Cached, write-through. Writes go to cache and memory simultaneously.
        WriteThrough,
        /// Caching disabled. All accesses go directly to the memory bus.
        /// Required for MMIO regions.
        Uncacheable,
        /// Write-combining. Multiple writes coalesced before hitting the bus.
        /// Optimal for framebuffers and PCIe BAR write streaming.
        WriteCombining,
        /// Write-protected. Reads cached; writes faulted (x86 specific).
        WriteProtect,
    }

    // ── Virtual memory protection flags ───────────────────────────────────────

    /// Protection attributes for a virtual memory mapping.
    ///
    /// Designed for use as a bitflag-style struct rather than an enum, since
    /// the three dimensions (readable, writable, executable) are orthogonal.
    /// Not all combinations are valid on all architectures (W^X enforcement
    /// is hardware-mandated on some; merely conventional on others).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct MemoryProtection {
        pub read:    bool,
        pub write:   bool,
        pub execute: bool,
    }

    impl MemoryProtection {
        pub const READ_ONLY:    Self = Self { read: true,  write: false, execute: false };
        pub const READ_WRITE:   Self = Self { read: true,  write: true,  execute: false };
        pub const READ_EXECUTE: Self = Self { read: true,  write: false, execute: true  };
        pub const KERNEL_CODE:  Self = Self::READ_EXECUTE;
        pub const KERNEL_DATA:  Self = Self::READ_WRITE;
        pub const NONE:         Self = Self { read: false, write: false, execute: false };

        /// Returns `true` if the combination is safe under strict W^X policy.
        /// (Writable+Executable simultaneously is never W^X safe.)
        #[inline]
        pub const fn is_wx_safe(self) -> bool {
            !(self.write && self.execute)
        }
    }

    // ── Memory allocation error ────────────────────────────────────────────────

    /// Reason a physical or virtual memory allocation request failed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum AllocError {
        /// No free region of the requested size exists.
        OutOfMemory,
        /// The requested alignment could not be satisfied.
        AlignmentUnsatisfied,
        /// The requested address range overlaps an existing mapping.
        RegionOverlap,
        /// The address or size was not aligned to the required page boundary.
        UnalignedAddress,
        /// The requested size was zero, which is nonsensical.
        ZeroSize,
        /// An internal data structure (e.g. page table) could not be allocated
        /// to satisfy the mapping.
        PageTableAllocationFailed,
    }

    // ── Useful size constants ──────────────────────────────────────────────────

    pub const KIB: u64 = 1024;
    pub const MIB: u64 = 1024 * KIB;
    pub const GIB: u64 = 1024 * MIB;
    pub const TIB: u64 = 1024 * GIB;

    pub const PAGE_SIZE_4K: u64 = PageSize::Kib4.bytes();
    pub const PAGE_SIZE_2M: u64 = PageSize::Mib2.bytes();
    pub const PAGE_SIZE_1G: u64 = PageSize::Gib1.bytes();

    /// Canonical x86-64 higher-half direct map base used by most bootloaders
    /// (Limine default). Not universal — verify against your bootloader.
    pub const X86_64_HHDM_DEFAULT: u64 = 0xffff_8000_0000_0000;

    /// Maximum addressable physical memory on x86-64 with 4-level paging
    /// and the default 48-bit physical address space.
    pub const X86_64_MAX_PHYS_4LEVEL: u64 = 1 << 46; // 64 TiB

    /// Maximum addressable physical memory on x86-64 with 5-level paging
    /// (LA57, 57-bit virtual, 52-bit physical).
    pub const X86_64_MAX_PHYS_5LEVEL: u64 = 1 << 52; // 4 PiB
}

// ═══════════════════════════════════════════════════════════════════════════════
// § privilege — CPU Privilege Levels
// ═══════════════════════════════════════════════════════════════════════════════

pub mod cpu_privilege {
    /// CPU privilege level abstraction.
    ///
    /// The concrete vocabulary varies dramatically by ISA:
    /// - x86-64: rings 0–3 (only 0 and 3 are used in practice)
    /// - AArch64: EL0–EL3 (application, kernel, hypervisor, secure monitor)
    /// - RISC-V: U/S/M modes (user, supervisor, machine)
    ///
    /// This enum provides a portable semantic layer. Map to ISA-specific
    /// encodings in your architecture-specific code.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
    pub enum CPUPrivilegeLevel {
        /// Highest privilege. Kernel / supervisor / machine mode.
        /// All instructions available. Unrestricted hardware access.
        /// x86: Ring 0 / AArch64: EL1 (or EL2/3) / RISC-V: M-mode (or S-mode).
        Kernel,

        /// Hypervisor / virtualisation extension privilege.
        /// x86: VMX root / AArch64: EL2 / RISC-V: HS-mode.
        /// Not relevant for non-hypervisor kernels.
        Hypervisor,

        /// Secure monitor / trust zone.
        /// AArch64: EL3. Not applicable on most architectures.
        SecureMonitor,

        /// Unprivileged user mode. Memory access gated by MMU.
        /// x86: Ring 3 / AArch64: EL0 / RISC-V: U-mode.
        User,
    }

    impl CPUPrivilegeLevel {
        /// x86-64 ring number (0–3). Returns `None` for levels with no x86
        /// direct equivalent (hypervisor maps to VMX root which is ring 0).
        #[inline]
        pub const fn x86_ring(self) -> Option<u8> {
            match self {
                Self::Kernel        => Some(0),
                Self::Hypervisor    => Some(0), // VMX root runs at ring 0
                Self::SecureMonitor => None,     // no x86 equivalent
                Self::User          => Some(3),
            }
        }

        /// AArch64 Exception Level.
        #[inline]
        pub const fn aarch64_el(self) -> Option<u8> {
            match self {
                Self::User          => Some(0),
                Self::Kernel        => Some(1),
                Self::Hypervisor    => Some(2),
                Self::SecureMonitor => Some(3),
            }
        }

        /// `true` if this level may access privileged CPU instructions and
        /// unrestricted physical memory (from the CPU's perspective — the VMM
        /// may impose further restrictions via second-stage paging).
        #[inline]
        pub const fn is_privileged(self) -> bool {
            !matches!(self, Self::User)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § interrupts — Interrupt Models, Trigger Modes, Delivery
// ═══════════════════════════════════════════════════════════════════════════════

pub mod interrupts {
    /// Interrupt controller architecture.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum InterruptController {
        /// Intel 8259A Programmable Interrupt Controller (legacy PC).
        /// Two chips cascaded, 15 usable IRQ lines, fixed vector offset.
        Pic8259,
        /// x86 Advanced Programmable Interrupt Controller.
        /// Per-CPU local APIC + I/O APIC(s). Supports 256 vectors,
        /// IRQ redirection, and MSIs.
        Apic,
        /// x2APIC — APIC accessed via MSRs rather than MMIO. Required for
        /// >255 CPU topologies.
        X2Apic,
        /// ARM Generic Interrupt Controller v2. Distributor + CPU interface.
        GicV2,
        /// ARM GICv3/v4. Redistributors per CPU; supports LPIs via ITS.
        GicV3,
        /// RISC-V Platform-Level Interrupt Controller.
        Plic,
        /// RISC-V Advanced Interrupt Architecture (AIA). Successor to PLIC.
        RiscVAia,
        /// Purely software-managed (no hardware interrupt controller).
        /// Polling or timer-only environments.
        None,
    }

    /// How an interrupt line is triggered electrically.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum TriggerMode {
        /// Rising-edge triggered. Interrupt fires on the 0→1 transition.
        EdgeRising,
        /// Falling-edge triggered. Interrupt fires on the 1→0 transition.
        EdgeFalling,
        /// Level-triggered, active high. Interrupt fires while line is high.
        LevelHigh,
        /// Level-triggered, active low. Interrupt fires while line is low.
        LevelLow,
    }

    /// Interrupt delivery mode (APIC / GIC terminology).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum DeliveryMode {
        /// Deliver to the target CPU as a normal (vectored) interrupt.
        Fixed,
        /// Deliver to the CPU with the lowest current priority.
        LowestPriority,
        /// System Management Interrupt. Causes SMM entry on x86.
        Smi,
        /// Non-maskable interrupt. Not affected by the IF flag.
        Nmi,
        /// Deliver as an INIT signal (APIC). Resets the target CPU.
        Init,
        /// Start-up IPI (APIC). Delivers the startup vector to an AP.
        StartUp,
        /// External interrupt (8259 compatibility mode).
        ExtInt,
    }

    /// Category of interrupt/exception by the CPU's own classification.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ExceptionKind {
        /// Fault: reported before the instruction completes. The saved RIP
        /// points to the faulting instruction; it can be retried after
        /// the fault is resolved (e.g. page fault → map page → iret).
        Fault,
        /// Trap: reported after the instruction completes. The saved RIP
        /// points to the next instruction (e.g. breakpoint, overflow).
        Trap,
        /// Abort: cannot be resumed. Saved state may be unreliable.
        /// (e.g. double fault, machine check).
        Abort,
        /// Hardware IRQ: asynchronous, not tied to any particular instruction.
        Interrupt,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § bus — Peripheral Bus Types
// ═══════════════════════════════════════════════════════════════════════════════

pub mod bus {
    /// System/peripheral bus or interconnect type.
    ///
    /// Used when enumerating devices, writing drivers, or classifying DMA
    /// sources. Not exhaustive — the embedded world has hundreds of proprietary
    /// buses; only those commonly encountered in OS-development contexts are
    /// included.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum BusKind {
        /// PCI (Conventional PCI, 32/64-bit parallel). Legacy.
        Pci,
        /// PCI Express (PCIe). Serial, lane-based. Ubiquitous.
        PciExpress,
        /// USB 1.1 / 2.0 (OHCI/UHCI/EHCI host).
        Usb2,
        /// USB 3.x (xHCI host).
        Usb3,
        /// I²C (Inter-Integrated Circuit). Two-wire, low-speed.
        I2c,
        /// SPI (Serial Peripheral Interface). Four-wire, synchronous.
        Spi,
        /// UART / serial port.
        Uart,
        /// Industry Standard Architecture (ISA). x86 legacy 8/16-bit bus.
        Isa,
        /// Enhanced ISA.
        Eisa,
        /// Low Pin Count bus. Modern x86 replacement for ISA (Super I/O chips).
        Lpc,
        /// SMBus (System Management Bus). I²C-compatible; used for SPD, EC.
        SmBus,
        /// AMBA APB (Advanced Peripheral Bus). ARM embedded.
        AmbApb,
        /// AMBA AHB (Advanced High-performance Bus). ARM embedded.
        AmbaAhb,
        /// AMBA AXI (Advanced eXtensible Interface). High-bandwidth ARM.
        AmbaAxi,
        /// MMIO-mapped device (no bus protocol; direct physical address).
        Mmio,
        /// Platform device (Device Tree / ACPI enumerated, no discovery bus).
        Platform,
        /// NVMe (Non-Volatile Memory Express). PCIe-attached storage protocol.
        Nvme,
        /// SATA (Serial ATA). Block storage.
        Sata,
        /// SD/MMC (Secure Digital / MultiMediaCard).
        SdMmc,
        /// CAN bus. Automotive and industrial control.
        Can,
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § display — Framebuffer / Display Formats
// ═══════════════════════════════════════════════════════════════════════════════

pub mod display {
    /// Pixel format / colour representation of a framebuffer.
    ///
    /// Component order is listed MSB to LSB within a pixel word.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[non_exhaustive]
    pub enum PixelFormat {
        /// 32-bit: Red[7:0] Green[15:8] Blue[23:16] padding[31:24].
        Rgb32,
        /// 32-bit: Blue[7:0] Green[15:8] Red[23:16] padding[31:24].
        Bgr32,
        /// 32-bit: Alpha[7:0] Red[15:8] Green[23:16] Blue[31:24].
        Argb32,
        /// 32-bit: Blue[7:0] Green[15:8] Red[23:16] Alpha[31:24].
        Bgra32,
        /// 24-bit packed RGB. No padding byte.
        Rgb24,
        /// 24-bit packed BGR. No padding byte.
        Bgr24,
        /// 16-bit RGB565: R[15:11] G[10:5] B[4:0].
        Rgb565,
        /// 8-bit indexed (palette-based).
        Indexed8,
        /// 8-bit greyscale.
        Grey8,
        /// Reported by firmware but format not decoded / not yet recognised.
        Unknown,
    }

    impl PixelFormat {
        /// Bits per pixel for this format.
        #[inline]
        pub const fn bpp(self) -> u8 {
            match self {
                Self::Rgb32 | Self::Bgr32 | Self::Argb32 | Self::Bgra32 => 32,
                Self::Rgb24 | Self::Bgr24 => 24,
                Self::Rgb565 => 16,
                Self::Indexed8 | Self::Grey8 => 8,
                Self::Unknown => 0,
            }
        }

        /// Bytes per pixel (rounded up). Returns `0` for `Unknown`.
        #[inline]
        pub const fn bytes_per_pixel(self) -> u8 {
            (self.bpp() + 7) / 8
        }

        /// `true` if the format has an explicit alpha channel.
        #[inline]
        pub const fn has_alpha(self) -> bool {
            matches!(self, Self::Argb32 | Self::Bgra32)
        }
    }

    /// Display rotation relative to natural (landscape) orientation.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum Rotation {
        /// No rotation. Natural landscape orientation.
        Degrees0,
        /// 90° clockwise.
        Degrees90,
        /// 180°. Upside down.
        Degrees180,
        /// 270° clockwise (90° counter-clockwise).
        Degrees270,
    }

    impl Rotation {
        /// Returns the opposite rotation (180° offset).
        #[inline]
        pub const fn flipped(self) -> Self {
            match self {
                Self::Degrees0   => Self::Degrees180,
                Self::Degrees90  => Self::Degrees270,
                Self::Degrees180 => Self::Degrees0,
                Self::Degrees270 => Self::Degrees90,
            }
        }

        /// Returns `true` if width and height are transposed relative to the
        /// unrotated framebuffer (i.e. for 90° and 270°).
        #[inline]
        pub const fn transposes_dimensions(self) -> bool {
            matches!(self, Self::Degrees90 | Self::Degrees270)
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// § version
// ═══════════════════════════════════════════════════════════════════════════════

/// Crate version, matching `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
