# Linux Command Implementation Reference
## x86_64 Hobby Kernel — Rust / Limine

> Commands are grouped by the kernel subsystem they require. Implement the subsystems
> in order; the command groups follow naturally. Your kernel currently has: heap (buddy +
> TLSF), VMM, IDT/PIC, GDT/TSS, framebuffer. Everything below that line is future work.
>
> Each entry states the **syscall(s)** the shell command ultimately invokes, the
> **kernel subsystem(s)** that must exist before it can work, and a plain description
> of what the command does from a user perspective.

---

## Tier 0 — Already Implementable (No Process Model Required)

These run in ring 0 as kernel built-ins before you have a scheduler or syscall interface.
They validate your existing subsystems interactively via a serial or framebuffer REPL.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `help` | none | none | Lists available commands and a one-line description of each. Pure text output. The canonical first command in every hobby kernel REPL. |
| `clear` | none | framebuffer | Fills the framebuffer with the background color and resets the cursor position. |
| `echo` | none | none | Writes its arguments to the output device. Validates argument parsing in your command dispatcher. |
| `version` | none | none | Prints kernel name, version string, build timestamp, and compiler version. |
| `halt` | none | x86 HLT | Disables interrupts and executes HLT in a loop. The controlled shutdown path before you have ACPI. |
| `reboot` | none | ACPI / port 0x64 | Triggers a system reset via the PS/2 keyboard controller reset line (write 0xFE to port 0x64) or the ACPI FADT reset register. |
| `meminfo` | none | buddy allocator | Reports total usable physical memory, free pages, allocated pages, and fragmentation statistics from the buddy's internal state. |
| `heapinfo` | none | TLSF heap | Reports TLSF heap usage: total capacity, bytes allocated, bytes free, largest free block. |
| `mmap` | none | VMM | Dumps the active virtual memory map: each mapped range, its physical backing, and page-table flags (present, writable, executable, cache policy). |
| `irqstats` | none | interrupt subsystem | Prints the per-vector delivery counters from `stats.rs`. Confirms the timer is firing and spurious IRQs are not accumulating. |
| `lspic` | none | PIC (pic.rs) | Reads and displays the 8259 PIC's IMR (Interrupt Mask Register) and ISR (In-Service Register) for both master and slave. |

---

## Tier 1 — Process Model Required

These require `fork`/`exec`, a scheduler, and a minimal virtual address space per process.

### Process and Session Management

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `ps` | `getpid`, `waitpid`, process table scan | process table, scheduler | Lists running processes: PID, PPID, state, priority, and command name. |
| `kill` | `kill` | signals, process table | Sends a signal to a process by PID. `kill -9 <pid>` sends SIGKILL; `kill -15` sends SIGTERM. Requires your signal delivery mechanism. |
| `sleep` | `nanosleep` or `clock_nanosleep` | scheduler, PIT/APIC timer | Suspends the calling process for a specified duration. Requires the scheduler to block and unblock on timer events. |
| `wait` | `wait` / `waitpid` | process table, signals | Blocks the shell until a child terminates, then reports its exit status. Required for sequential script execution. |
| `exit` | `exit` / `exit_group` | process table | Terminates the current process. The kernel must reap it, free its address space, and notify the parent via SIGCHLD. |
| `true` | `exit(0)` | process table | Exits immediately with status 0. Used in shell conditionals to force a success result. |
| `false` | `exit(1)` | process table | Exits immediately with status 1. Used in shell conditionals to force a failure result. |
| `env` | `execve` (environment inspection) | process table | Prints environment variables. Requires `execve` to propagate `envp[]` correctly. |
| `nohup` | `signal(SIGHUP, SIG_IGN)`, `execve` | signals, process model | Runs a command immune to hangup signals. Keeps processes alive after terminal disconnect. |
| `nice` | `setpriority`, `execve` | scheduler, process model | Runs a command with a modified scheduling priority. Requires the scheduler to honor a priority field per process. |
| `renice` | `setpriority` | scheduler, process table | Changes the scheduling priority of a running process. |
| `timeout` | `alarm` / `timer_create`, `execve` | signals, timer, process model | Runs a command and kills it if it runs longer than a specified duration. |

### Job Control

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `fg` | `tcsetpgrp`, `kill(SIGCONT)` | signals, process groups, TTY | Moves a background or stopped job to the foreground and resumes it. Requires process groups and terminal ownership. |
| `bg` | `kill(SIGCONT)` | signals, process groups | Resumes a stopped background job without bringing it to the foreground. |
| `jobs` | none (shell built-in) | shell job table | Lists jobs managed by the current shell: job number, state, and command. Entirely shell-internal. |

---

## Tier 2 — VFS and Filesystem Required

These require a virtual filesystem layer with at minimum an in-memory tmpfs or initrd-backed
filesystem. No block device driver is necessary for the initial versions.

### File and Directory Inspection

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `ls` | `openat`, `getdents64`, `stat` | VFS, directory entries | Lists directory contents. With `-l`, shows permissions, link count, owner, size, and mtime. The most frequently invoked command; implement it early. |
| `pwd` | `getcwd` | VFS, process CWD | Prints the absolute path of the current working directory. Requires the kernel to track each process's CWD as a VFS dentry. |
| `cd` | `chdir` / `fchdir` | VFS, process CWD | Changes the process's current working directory. Shell built-in; single `chdir` syscall. |
| `stat` | `stat` / `fstat` / `statx` | VFS, inodes | Displays detailed inode metadata: size, block count, inode number, link count, permissions, all three timestamps. |
| `file` | `open`, `read`, `stat` | VFS | Identifies a file's type by inspecting its magic bytes. Confirms your filesystem stores files correctly. |
| `find` | `openat`, `getdents64`, `fstatat` | VFS, recursive traversal | Traverses a directory tree and prints paths matching given criteria. Heavy VFS exerciser. |
| `du` | `openat`, `getdents64`, `fstatat` | VFS, block accounting | Reports disk usage of files and directories. Requires block count tracking in inodes. |
| `df` | `statfs` / `statvfs` | VFS, filesystem metadata | Reports filesystem capacity, used space, and available space per mount point. |
| `tree` | `openat`, `getdents64` | VFS | Prints a directory tree recursively in visual hierarchy. Simpler than `find`; a good early VFS test. |

### File Creation and Manipulation

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `touch` | `open` (O_CREAT), `utimensat` | VFS, timestamp updates | Creates an empty file if it does not exist, or updates its timestamps if it does. |
| `mkdir` | `mkdirat` | VFS, directory creation | Creates directories. With `-p`, creates intermediate parents as needed. |
| `rmdir` | `rmdir` | VFS | Removes an empty directory. Fails if the directory has entries. |
| `rm` | `unlinkat` | VFS, link count management | Removes files by unlinking them. With `-r`, recursive. With `-f`, suppresses errors for nonexistent files. |
| `mv` | `renameat2` | VFS | Renames or moves a file or directory. Cross-filesystem moves require copy + unlink. |
| `cp` | `open`, `read`, `write`, `stat` | VFS | Copies files or directory trees. With `-p`, preserves timestamps and permissions. |
| `ln` | `linkat` (hard), `symlinkat` (soft) | VFS, symlink support | Creates hard or symbolic links. Hard links require link count management; symlinks need a separate inode type. |
| `truncate` | `truncate` / `ftruncate` | VFS, file extent management | Sets a file's size exactly, either extending (zero-filling) or shrinking it. |
| `install` | `open`, `write`, `fchmod`, `fchown` | VFS, permissions | Copies files and sets permissions in one operation. Commonly used in build systems. |
| `basename` | none | none | Strips the directory portion and optional suffix from a path string. Pure string manipulation; no syscall needed. |
| `dirname` | none | none | Strips the last component from a path string. Pure string manipulation. |
| `readlink` | `readlinkat` | VFS, symlinks | Prints the target of a symbolic link. Requires symlink support in your VFS. |
| `realpath` | `realpath` / manual resolution | VFS | Resolves all symlinks and `..` components and prints the canonical absolute path. |

### File Content

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `cat` | `open`, `read`, `write` | VFS | Reads files and writes their contents to stdout. The simplest non-trivial file command. |
| `head` | `open`, `read` | VFS | Prints the first N lines (default 10) of a file. |
| `tail` | `open`, `read`, `lseek` | VFS | Prints the last N lines of a file. With `-f`, follows appended data — requires `inotify` or polling. |
| `less` / `more` | `open`, `read`, `ioctl` | VFS, TTY | Paginated file viewer. Requires terminal raw mode and ANSI cursor control. |
| `wc` | `open`, `read` | VFS | Counts lines, words, and bytes. The classic "does read() work correctly" validation. |
| `sort` | `open`, `read`, `write` | VFS | Reads lines, sorts them, writes the result. May require temp files for large inputs. |
| `uniq` | `open`, `read`, `write` | VFS | Filters or reports adjacent duplicate lines. Usually piped after `sort`. |
| `cut` | `open`, `read`, `write` | VFS | Extracts fields or byte ranges from each line of input. |
| `paste` | `open`, `read`, `write` | VFS | Merges lines from multiple files side by side, separated by a delimiter. |
| `join` | `open`, `read`, `write` | VFS | Joins lines from two sorted files on a common field. Relational join for text files. |
| `tee` | `open`, `read`, `write` | VFS, pipes | Reads stdin and writes to both stdout and a file simultaneously. Requires pipe support. |
| `grep` | `open`, `read` | VFS | Searches files for lines matching a pattern. Requires a regex engine in userspace. |
| `sed` | `open`, `read`, `write` | VFS | Stream editor: applies substitution and deletion commands to each line of input. |
| `awk` | `open`, `read`, `write` | VFS | Pattern-action text processor. More capable than sed; requires a small interpreter. |
| `diff` | `open`, `read` | VFS | Compares two files line by line and outputs their differences in a standard format. |
| `patch` | `open`, `read`, `write` | VFS | Applies a diff patch to a file or directory tree. |
| `tr` | `read`, `write` | pipes/TTY | Translates or deletes characters from stdin. Useful for case conversion and whitespace normalization. |
| `strings` | `open`, `read` | VFS | Extracts printable character sequences from a binary file. Useful for inspecting ELF binaries. |
| `od` | `open`, `read` | VFS | Dumps file contents in octal, hexadecimal, or other formats. Essential for inspecting binary file correctness. |
| `xxd` | `open`, `read` | VFS | Hex dump utility; also converts hex back to binary. More readable than `od` for most purposes. |

### Permissions and Identity

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `chmod` | `fchmodat` | VFS, permission bits | Changes file permission bits. Requires your inode to store and enforce UNIX permissions. |
| `chown` | `fchownat` | VFS, UID/GID model | Changes the owning user and group of a file. Requires a user/group identity model. |
| `umask` | `umask` | process credentials | Sets the file creation mask ANDed out of new file permissions. Shell built-in. |
| `id` | `getuid`, `getgid`, `getgroups` | process credentials | Prints the current user's UID, GID, and supplementary group memberships. |
| `whoami` | `getuid`, then `/etc/passwd` lookup | VFS, process credentials | Prints the username corresponding to the current UID. |
| `groups` | `getgroups` | process credentials | Lists the groups the current user belongs to. |
| `su` | `setuid`, `setgid`, `execve` | process credentials, PAM/shadow | Switches the running user identity. Requires a credential validation mechanism. |
| `sudo` | `setuid`, `execve` | process credentials, policy engine | Runs a command as another user (typically root) per a policy file. Substantially more complex than `su`. |

---

## Tier 3 — Block Device and Full Filesystem

These require a real block device driver (ATA, NVMe, or virtio-blk) and a filesystem
implementation (ext2 is the traditional starting point; FAT32 is simpler).

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `mount` | `mount` | VFS, block driver, filesystem driver | Attaches a filesystem on a block device to a directory mount point in the VFS tree. |
| `umount` | `umount2` | VFS, filesystem driver | Detaches a mounted filesystem. Must flush dirty buffers and reject if files are open. |
| `fsck` | direct block device access | block driver, filesystem internals | Checks and repairs filesystem consistency: superblock, inode/block bitmaps, directory entries. |
| `mkfs` | direct block device access | block driver | Formats a block device with a filesystem, writing the on-disk structure from scratch. |
| `blkid` | `ioctl`, block reads | block driver, partition table | Identifies filesystem type, label, and UUID by reading the device's superblock. |
| `lsblk` | sysfs or block device registry | block driver | Lists block devices, their sizes, and partition layout. |
| `fdisk` / `gdisk` | direct block device access | block driver | Interactive partition table editor. Reads and writes MBR or GPT partition tables. |
| `dd` | `open`, `read`, `write`, `lseek` | block driver, VFS | Copies raw data between files or devices with configurable block size. Standard tool for writing disk images and testing raw block I/O. |
| `sync` | `sync` / `fsync` | VFS, buffer cache, block driver | Flushes all pending writes from the buffer cache to the block device. Critical for durability. |
| `hdparm` | `ioctl` (ATA commands) | ATA block driver | Queries and sets ATA hard disk parameters: read-ahead, DMA mode, power management. |

---

## Tier 4 — Networking Stack

Requires a NIC driver, an IP stack (TCP/UDP/ICMP), and socket syscalls.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `ping` | `socket`, `sendto`, `recvfrom` | ICMP, IP, NIC driver | Sends ICMP Echo Requests and measures round-trip time. Implement ICMP before TCP. |
| `ip` / `ifconfig` | `socket`, `ioctl` (SIOCGIFADDR etc.) | network interface registry | Configures and displays network interface addresses, routes, and link state. |
| `route` | `socket`, `ioctl` (SIOCADDRT) | routing table | Displays or modifies the kernel's IP routing table. |
| `netstat` | `socket`, `/proc/net` or `getsockopt` | TCP/UDP socket table | Displays active connections, listening sockets, and network statistics. |
| `ss` | `socket`, Netlink | socket table, Netlink | Modern replacement for `netstat`. Uses the Netlink socket interface to query socket state. |
| `wget` / `curl` | `socket`, `connect`, `send`, `recv` | TCP, DNS, HTTP | Fetches resources over HTTP. Requires TCP and a stub DNS resolver or hardcoded IPs. |
| `nc` (netcat) | `socket`, `connect` / `bind`, `send`, `recv` | TCP and/or UDP | Opens raw TCP or UDP connections and pipes stdin/stdout to the socket. The most useful network debugging tool after ping. |
| `telnet` | `socket`, `connect`, `send`, `recv` | TCP | Connects to a remote host over raw TCP with a simple line-oriented protocol. Obsolete for production; useful for testing your TCP stack. |
| `nslookup` / `dig` | `socket`, `sendto`, `recvfrom` | UDP, DNS resolver | Queries a DNS server and displays the response. Requires a DNS packet encoder/decoder. |
| `traceroute` | `socket`, `setsockopt` (TTL), ICMP | ICMP, UDP, IP | Maps the network path to a host by sending probes with incrementing TTL values and recording ICMP TTL-Exceeded responses. |
| `arp` | `ioctl` (SIOCDARP etc.), ARP table | ARP, NIC driver | Displays and manipulates the ARP cache mapping IP addresses to MAC addresses. |
| `tcpdump` | `socket` (AF_PACKET), raw sockets | raw socket, NIC driver | Captures and displays network packets. Requires promiscuous mode and a raw socket interface. |
| `ssh` | `socket`, `connect`, crypto primitives | TCP, crypto library | Secure remote shell. Requires a full cryptographic library. Not a first-year hobby kernel target. |
| `scp` | same as ssh | TCP, crypto, VFS | Copies files over an SSH connection. Depends entirely on a working SSH implementation. |

---

## Tier 5 — Procfs, Sysfs, and Kernel Introspection

Requires virtual filesystems that synthesize content from kernel data structures on read.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `uname` | `uname` | kernel version struct | Prints kernel name, hostname, release, machine type, and OS. Single syscall populating a `utsname` struct. |
| `uptime` | `clock_gettime(CLOCK_BOOTTIME)` | monotonic timer | Reports how long the system has been running and load averages. Requires a boot timestamp. |
| `free` | `/proc/meminfo` or `sysinfo` | procfs or `sysinfo` syscall | Displays total, used, and free physical memory. `sysinfo` is the simplest implementation path. |
| `top` / `htop` | `/proc/[pid]/stat`, `clock_gettime` | procfs, scheduler | Interactive process monitor: CPU and memory usage per process, updated periodically. Requires per-process CPU time accounting. |
| `dmesg` | `syslog` or `/dev/kmsg` | kernel log ring buffer | Prints the kernel message ring buffer. Requires a circular log buffer capturing all kernel print output since boot. |
| `lscpu` | `/proc/cpuinfo` or CPUID | procfs, CPUID | Displays CPU topology, vendor, model, feature flags, and cache sizes. Feature flags come from the CPUID instruction. |
| `lsmem` | `/sys/devices/system/memory` or `/proc/iomem` | sysfs or procfs | Reports physical memory ranges and types. Essentially a formatted view of your boot memory map. |
| `lsmod` | `/proc/modules` | module subsystem, procfs | Lists loaded kernel modules. Only relevant if you implement loadable modules. |
| `lspci` | direct port I/O or `/proc/bus/pci` | PCI bus enumeration | Lists PCI devices and their vendor/device IDs. Requires PCI configuration space access (port 0xCF8/0xCFC). |
| `lsusb` | USB host controller, `/sys/bus/usb` | USB host driver, sysfs | Lists connected USB devices. Requires a USB host controller driver (EHCI, xHCI). |
| `vmstat` | `/proc/vmstat` or `sysinfo` | procfs, VM subsystem | Reports virtual memory statistics: page faults, swapped pages, I/O activity. |
| `iostat` | `/proc/diskstats` | procfs, block driver | Reports I/O statistics per block device: reads/writes per second, throughput, await time. |
| `mpstat` | `/proc/stat` | procfs, scheduler | Reports CPU utilization statistics per processor. Requires per-CPU time accounting. |
| `cpufreq-info` | `/sys/devices/system/cpu` | sysfs, ACPI/MSR | Reports CPU frequency scaling information. Requires reading CPUID or MSRs. |

---

## Tier 6 — TTY, Shell, and User Environment

Requires a TTY layer, line discipline, and a functioning userspace shell binary.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `stty` | `tcgetattr`, `tcsetattr` | TTY, line discipline | Reads and modifies terminal settings: echo mode, canonical vs raw, special character assignments. |
| `tty` | `isatty`, `ttyname` | TTY | Prints the filename of the terminal connected to stdin. |
| `reset` | `tcsetattr`, ANSI escapes | TTY | Resets the terminal to a sane state. Invaluable when a program crashes and leaves the terminal in raw mode. |
| `tput` | `tigetstr` (terminfo) | terminfo database | Outputs terminal capability strings: cursor movement, color codes, screen clearing. |
| `which` | `access`, `stat` (PATH search) | VFS, process environment | Locates a command in PATH and prints its full path. |
| `type` | PATH search (shell built-in) | shell | Indicates whether a name is a shell built-in, function, alias, or external binary. |
| `alias` | none | shell | Defines a short name for a longer command string. Entirely shell-internal. |
| `unalias` | none | shell | Removes a previously defined alias. Shell-internal. |
| `history` | none (file I/O for persistence) | shell, optional VFS | Displays the shell's command history list. |
| `source` / `.` | none (shell built-in) | shell | Executes a script file in the current shell process, not a subprocess. Shell-internal. |
| `export` | none (shell built-in) | shell, process environment | Marks a shell variable for export to child processes via `execve`'s `envp[]`. |
| `set` | none (shell built-in) | shell | Displays or modifies shell options and positional parameters. Shell-internal. |
| `unset` | none (shell built-in) | shell | Removes a shell variable or function. Shell-internal. |
| `read` | `read` syscall | TTY, shell | Reads a line from stdin into a shell variable. Shell built-in around the `read` syscall. |
| `printf` | `write` | TTY | Formats and prints text with format string support. More portable than `echo`. |
| `man` | `open`, `read`, pager | VFS, pager | Displays manual pages. For a hobby kernel, plain text files served by `less` are adequate. |
| `sh` / `bash` | `fork`, `execve`, `pipe`, `dup2`, `waitpid` | process model, signals, pipes, VFS | The shell itself. Requires virtually the entire kernel. A minimal POSIX shell is achievable; full bash is a multi-year project. |

---

## Tier 7 — IPC and Synchronization Primitives

These require inter-process communication mechanisms beyond simple pipes.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `ipcs` | `msgctl`, `semctl`, `shmctl` | SysV IPC (message queues, semaphores, shared memory) | Lists active IPC resources: message queues, semaphore sets, and shared memory segments. |
| `ipcrm` | `msgctl(IPC_RMID)`, `semctl(IPC_RMID)`, `shmctl(IPC_RMID)` | SysV IPC | Removes a specified IPC resource. Necessary to clean up orphaned IPC objects after process crashes. |
| `mkfifo` | `mknodat` (S_IFIFO) | VFS, named pipes | Creates a named pipe (FIFO) in the filesystem. Allows unrelated processes to communicate via the VFS. |
| `flock` | `flock` | VFS, advisory locking | Acquires an advisory lock on a file. Used for inter-process mutual exclusion around shared file access. |

---

## Tier 8 — Advanced and Debugging Tools

Implement these after the above tiers are solid. High development value, deep prerequisites.

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `strace` | `ptrace` | ptrace, process model | Intercepts and logs every syscall made by a target process. Requires `ptrace` — the most complex single syscall to implement correctly. |
| `ltrace` | `ptrace`, dynamic linker hooks | ptrace, dynamic linker | Intercepts library calls in a target process. Requires a working dynamic linker. |
| `gdb` (stub) | `ptrace`, `SIGTRAP`, debug registers | ptrace, signals | Remote debugging via GDB's RSP protocol. A GDB stub over serial can be implemented in the kernel without full `ptrace`. |
| `valgrind` | `ptrace` or binary translation | ptrace | Memory error detector and profiler. Operates by running the target in a sandboxed execution environment. Not a hobby kernel first-year target. |
| `ldd` | `open`, `read` (ELF parsing) | dynamic linker, VFS | Lists shared libraries required by an ELF binary. Irrelevant if you only run statically linked binaries. |
| `objdump` | `open`, `read` (ELF parsing) | VFS | Disassembles ELF binaries. More useful as a host-side cross-tool than a target-side command. |
| `readelf` | `open`, `read` (ELF parsing) | VFS | Displays detailed ELF file headers, section headers, symbol tables, and dynamic linking information. |
| `nm` | `open`, `read` (ELF symbol tables) | VFS | Lists symbols from an ELF binary's symbol table. |
| `addr2line` | `open`, `read` (DWARF parsing) | VFS, DWARF | Converts instruction addresses to source file and line numbers. Run it on the host against your kernel ELF for panic symbolization. |
| `size` | `open`, `read` (ELF parsing) | VFS | Reports the sizes of the text, data, and BSS segments of an ELF binary. |
| `time` | `clock_gettime`, `wait` | monotonic timer, process model | Measures wall-clock, user, and system time consumed by a command. Requires per-process CPU time accounting. |
| `watch` | `clock_nanosleep`, `fork`/`exec` | timer, process model | Runs a command repeatedly at a fixed interval and displays the output. Simple once you have sleep and exec. |
| `cron` / `at` | `clock_gettime`, `fork`, `execve` | timer, process model, VFS | Scheduled task execution. `at` runs once; `cron` runs on a repeating schedule. |
| `perf` | `perf_event_open`, PMU access | performance monitoring unit, process model | CPU performance counter profiling. Requires access to x86 PMU MSRs and a sampling interrupt handler. |
| `ftrace` | tracefs (`/sys/kernel/debug/tracing`) | kernel tracing infrastructure | Kernel function tracing framework. Requires instrumented function entry/exit points in the kernel. |

---

## Implementation Priority Order

Given your current kernel state (heap, VMM, interrupts, framebuffer — no process model):

1. **Serial/framebuffer REPL** → implement Tier 0 immediately to validate existing subsystems
2. **Syscall interface** (SYSCALL/SYSRET on x86_64) → prerequisite for any userspace binary
3. **Scheduler + fork/exec** → unlocks all of Tier 1
4. **tmpfs (in-memory VFS)** → unlocks Tier 2 without needing a block driver
5. **procfs** → unlocks most of Tier 5; the commands are trivial once the FS exists
6. **TTY / line discipline** → unlocks an interactive userspace shell (Tier 6)
7. **ATA or virtio-blk + ext2** → unlocks Tier 3 for persistent storage
8. **Network stack** → Tier 4; can be parallelized with Tier 3

---

*Compiled against your kernel's current state (heap + VMM + interrupt subsystem functional,
no process model or VFS). The Linux man-pages project and OSDev Wiki are the authoritative
references for syscall semantics. POSIX.1-2017 (IEEE Std 1003.1) is the authoritative
reference for command behavior.*

---

## How Real Operating Systems Structure Userland Utilities

Before deciding where commands live in your repository, it is worth understanding how
production operating systems solved the same problem — because the tradeoffs they made
are still present and visible in their source trees today.

---

### Linux — GNU Coreutils and the Busybox Alternative

Linux does not ship commands in the kernel repository at all. The kernel (`torvalds/linux`)
contains zero userspace utilities. Commands are provided by entirely separate projects:

**GNU Coreutils** (`git://git.sv.gnu.org/coreutils`) is the canonical source of `ls`,
`cat`, `cp`, `mv`, `rm`, `mkdir`, `chmod`, and ~100 others. Each command is a standalone
C source file under `src/` compiled into an independent binary. There is no shared `lib.rs`
equivalent — common functionality (argument parsing, error reporting, locale handling) is
provided by a library called `libcoreutils` compiled as an archive linked into each binary
at build time. The structure is:

```
coreutils/
├── src/
│   ├── ls.c         # compiled to /usr/bin/ls
│   ├── cat.c        # compiled to /usr/bin/cat
│   ├── cp.c         # compiled to /usr/bin/cp
│   └── ...
├── lib/             # shared routines: xmalloc, error(), quote(), etc.
│   ├── xmalloc.c
│   ├── error.c
│   └── ...
└── Makefile.am
```

Each binary is wholly independent at runtime. The shared library is a **build-time**
convenience, not a runtime dependency. This matters: if `ls` crashes, `cat` keeps working.

**Busybox** takes the diametrically opposite approach, motivated by embedded systems where
binary count and total disk footprint are hard constraints. All ~300 commands are compiled
into a **single binary**. The binary inspects `argv[0]` at startup and dispatches to the
correct command implementation. Symlinking `/bin/ls → /bin/busybox` is sufficient to
make `ls` work. The internal structure is:

```
busybox/
├── coreutils/
│   ├── ls.c         # compiled IN, not compiled TO
│   ├── cat.c
│   └── ...
├── shell/
│   └── ash.c        # the shell, also compiled in
├── libbb/           # shared routines used by all applets
│   ├── xfuncs.c
│   └── ...
├── include/
│   └── applets.h    # dispatch table: { "ls", ls_main, ... }
└── busybox.c        # main() → looks up argv[0] → calls ls_main()
```

The dispatch table is generated at build time from a list of enabled applets. Commands
are called `ls_main()`, `cat_main()`, etc. — functions, not programs.

**The key insight from Linux/Busybox:** the kernel repository contains none of this.
The separation is intentional and absolute. The kernel is compiled as one artifact;
userland is compiled as another. They communicate only through the syscall ABI.

---

### seL4 / Redox — Microkernel and "Everything is a Process" Approaches

**seL4** (the formally verified microkernel) takes separation furthest. The kernel itself
provides exactly four abstractions: capabilities, threads, IPC endpoints, and virtual
memory objects. It contains no drivers, no filesystem, no shell, no commands whatsoever.
Everything else — including the VFS, device drivers, and command-line utilities — runs as
userspace processes communicating via IPC. Their repository structure reflects this:

```
seL4/                    # kernel only — ~10k lines
seL4_tools/              # separate repo: build system
seL4-CAmkES-L4v-dockerfiles/  # environment
projects/sel4test/       # test suite, also separate
```

Commands and a shell are expected to be ported from an existing userland (typically
musl libc + busybox or a custom minimal environment). seL4 does not pretend to own that
problem.

**Redox OS** (the Rust OS most directly comparable to what you are building) structures
this as a Cargo workspace with userland commands as separate crates under a `cookbook`
package manager. Each command is its own `[[bin]]` entry in a crate:

```
redox/
├── kernel/              # the kernel crate
├── cookbook/            # package recipes (like ports/)
│   ├── bash/
│   ├── coreutils/
│   └── ...
└── relibc/              # Redox's C library (Rust + C)

# Utilities live in a separate repo: redox-os/coreutils
redox-os/coreutils/
├── Cargo.toml           # workspace
├── src/
│   ├── bin/
│   │   ├── ls.rs        # [[bin]] name = "ls"
│   │   ├── cat.rs       # [[bin]] name = "cat"
│   │   └── ...
│   └── lib.rs           # shared: arg parsing, error formatting
```

The `src/bin/` convention is standard Cargo: every `.rs` file under `src/bin/` becomes
an independently compiled binary. The `lib.rs` provides common utilities available to all
of them. This is the most directly applicable model for your kernel.

---

### xv6 — The Teaching OS

xv6 (MIT's teaching OS, used in 6.S081) is the closest in spirit to a hobby kernel at
your stage. It keeps everything in one repository, including both kernel and userland, but
draws a hard compile-time boundary between them:

```
xv6-riscv/
├── kernel/
│   ├── main.c
│   ├── proc.c
│   ├── vm.c
│   └── ...          # kernel sources, compiled to kernel.elf
├── user/
│   ├── ls.c         # each file compiles to a separate binary
│   ├── cat.c
│   ├── sh.c         # the shell
│   ├── ulib.c       # shared userspace library (printf, strcpy, etc.)
│   └── usys.S       # syscall stubs (ECALL wrappers)
└── Makefile         # builds kernel.elf, then each user/*.c separately
```

The Makefile compiles `user/ls.c` into a standalone ELF, links it against `ulib.c`
and `usys.S` (which provide the C library and syscall entry points), and packages the
result into a filesystem image that the kernel mounts at boot. The kernel and user
binaries are compiled with different flags, different linker scripts, and different
standard libraries.

The key architectural decision xv6 makes: **userland lives in the same repo as the
kernel during early development, but is compiled as a wholly separate artifact.** The
boundary is enforced by the build system, not by directory structure alone.

---

### Recommended Repository Structure for Your Kernel

Given your current state — Rust, Cargo, `no_std` kernel, no VFS yet — the most pragmatic
structure that does not require rearchitecting anything is:

```
my-kernel/                        # Cargo workspace root
├── Cargo.toml                    # [workspace] members = ["kernel", "ulib", "init", "commands"]
│
├── kernel/                       # your current kernel crate (no_std)
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── gdt.rs
│       ├── interrupts/
│       ├── memory/
│       └── writers/
│
├── ulib/                         # userspace standard library (no libc, your syscall wrappers)
│   ├── Cargo.toml                # std = false, links against kernel ABI
│   └── src/
│       ├── lib.rs                # pub mod syscall; pub mod io; pub mod args;
│       ├── syscall.rs            # raw SYSCALL/SYSRET wrappers (inline asm)
│       ├── io.rs                 # print!, eprintln!, buffered I/O over write()
│       ├── args.rs               # argv/argc parsing helpers
│       └── process.rs            # exit(), getpid(), etc.
│
├── init/                         # PID 1 — the first userspace process
│   ├── Cargo.toml                # [[bin]] name = "init"
│   └── src/
│       └── main.rs               # mounts filesystems, spawns shell, reaps zombies
│
└── commands/                     # all userland commands as one crate
    ├── Cargo.toml
    │   # [lib] — shared command utilities
    │   # [[bin]] name = "ls"
    │   # [[bin]] name = "cat"
    │   # [[bin]] name = "echo"
    │   # ... one [[bin]] entry per command
    └── src/
        ├── lib.rs                # shared: arg parsing, error formatting, table output
        ├── bin/
        │   ├── ls.rs             # fn main() using lib's helpers + ulib syscalls
        │   ├── cat.rs
        │   ├── echo.rs
        │   ├── ps.rs
        │   ├── kill.rs
        │   └── ...
        └── common/
            ├── fmt.rs            # column formatting, human-readable sizes
            ├── path.rs           # path manipulation without std::path
            └── error.rs          # consistent error reporting to stderr
```

**Why this structure, specifically:**

The `commands/` crate uses Cargo's native `src/bin/` multi-binary convention. Each file
under `src/bin/` compiles to an independent binary — no manual build system wiring. The
`lib.rs` provides shared utilities that are linked into each binary at compile time and
disappear at runtime (static linking). This is functionally identical to what Busybox
does, but with Cargo handling the dispatch.

`ulib/` is the equivalent of `usys.S` + `ulib.c` in xv6 — the bridge between your
userspace binaries and your kernel's syscall ABI. It must be a separate crate from
`kernel/` because `kernel/` is `no_std` compiled for ring 0, and `ulib/` is also `no_std`
but compiled for ring 3 with your kernel's syscall numbers. They share no code at runtime.

`init/` is separate because PID 1 has unique responsibilities (mounting the root
filesystem, reaping all orphaned processes, never exiting) that do not belong mixed
in with regular command implementations.

**The Tier 0 REPL is a temporary exception.** Until you have a process model and
syscall interface, your "commands" are just functions called from `kernel_main`. Keep
them in `kernel/src/repl/` as a module, not in the `commands/` crate — they are kernel
code, not userspace code, and mixing them into the userspace crate would blur the
ring-0/ring-3 boundary before you have built it.

---

## Additional Commands Not Previously Listed

These were omitted from the original tiers but are genuinely useful during kernel
development and early userspace bringup.

### Tier 0 Additions (REPL, No Process Model)

| Command | Subsystems | Description |
|---------|------------|-------------|
| `cpuid` | x86 CPUID | Executes CPUID with a specified leaf and prints EAX/EBX/ECX/EDX in hex. Invaluable for confirming what features the CPU reports to your kernel. |
| `rdmsr` | x86 MSR | Reads a Model-Specific Register by address and prints its value. Essential for debugging APIC base, EFER, and SYSCALL-related MSRs before they are wired up. |
| `wrmsr` | x86 MSR | Writes a value to an MSR. Use with extreme caution; an incorrect write to EFER or CR0 will triple-fault immediately. |
| `pagewalk` | VMM, page tables | Given a virtual address, walks the four-level x86_64 page table (PML4 → PDPT → PD → PT) and prints each entry's flags. The single most useful VMM debugging tool you can build. |
| `stacktrace` | stack unwinder | Walks the frame pointer chain from the current RSP and prints return addresses. Useful before you have a panic handler with real unwinding. |
| `serial` | serial driver | Reads or writes raw bytes to a UART port. Useful for testing serial I/O independently of the framebuffer. |

### Tier 1 Additions (Process Model)

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `pstree` | process table scan | process table, scheduler | Displays the process hierarchy as a tree rooted at PID 1. Immediately reveals orphan and zombie processes that `ps` presents ambiguously. |
| `taskset` | `sched_setaffinity`, `sched_getaffinity` | scheduler, SMP | Pins a process to a specific CPU core or set of cores. Only relevant once you have SMP; invaluable for debugging CPU-local data structures. |
| `setsid` | `setsid` | process groups, TTY | Creates a new session and makes the calling process its leader, detaching from the controlling terminal. Required for implementing daemon processes correctly. |

### Tier 2 Additions (VFS)

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `ls -la /proc` | `getdents64` | procfs | Not a new command, but the specific invocation that confirms procfs is mounted and populated correctly. The first thing you run after enabling procfs. |
| `mktemp` | `open` (O_CREAT \| O_EXCL), `getpid` | VFS | Creates a temporary file with a unique name. Prevents race conditions in scripts that need scratch space. |
| `install -d` | `mkdirat` | VFS | Creates a directory and all parents in one call, with specified permissions. Useful in build scripts and package installers. |
| `namei` | VFS path resolution | VFS, symlinks | Traces the path resolution of a filename, showing each component and any symlinks encountered. Directly exercises your VFS dentry resolution code. |
| `tee` | `read`, `write`, `open` | VFS, pipes | Reads stdin and writes simultaneously to stdout and one or more files. Requires pipe support to be useful; tests that two `write()` calls to different fds work correctly. |
| `hexdump` | `open`, `read` | VFS | Displays file contents in hexadecimal and ASCII side by side. More useful during filesystem bringup than `od` — the output format makes corrupted inode data immediately obvious. |

### Tier 5 Additions (Procfs / Kernel Introspection)

| Command | Syscalls | Subsystems | Description |
|---------|----------|------------|-------------|
| `cat /proc/interrupts` | `open`, `read` | procfs, interrupt subsystem | Displays per-IRQ delivery counts per CPU. Essentially your existing `irqstats` REPL command exposed through the VFS. Implement this first in procfs — the data already exists in `stats.rs`. |
| `cat /proc/buddyinfo` | `open`, `read` | procfs, buddy allocator | Shows the buddy allocator's free-list state per order per zone. The procfs equivalent of your `meminfo` REPL command. |
| `cat /proc/slabinfo` | `open`, `read` | procfs, slab allocator | Reports per-slab-cache statistics if you implement a slab layer above the buddy. Not relevant until you have typed object caches. |
| `cat /proc/self/maps` | `open`, `read` | procfs, VMM | Prints the virtual memory map of the current process. The procfs equivalent of your `mmap` REPL command, but scoped to a single process rather than the entire kernel address space. |
| `cat /proc/self/status` | `open`, `read` | procfs, process table | Prints process metadata: name, PID, PPID, UID, GID, VM sizes, and thread count. A one-stop sanity check for the process model. |

### Developer and Build Workflow Commands (Host-Side, Not Target-Side)

These run on your development machine, not inside your kernel. They are not userspace
commands you implement — they are the tools you use while implementing everything else.
Listed here because they appear constantly in OS development workflows and should be
in your toolchain from day one.

| Tool | Purpose |
|------|---------|
| `qemu-system-x86_64` | The primary emulator target. Boot your kernel image, expose a serial console (`-serial stdio`), and attach a GDB stub (`-s -S`) without physical hardware. |
| `gdb` (host) | Connect to QEMU's GDB stub. Load your kernel ELF with debug symbols (`file kernel.elf`), set breakpoints, inspect registers and memory. The most important debugging tool you have. |
| `addr2line` (host) | Convert a raw instruction address from a panic or triple-fault to a source file and line. Run it on your host against the kernel ELF: `addr2line -e kernel.elf 0xffffffff80012345`. |
| `objdump -d` (host) | Disassemble the compiled kernel ELF to verify the code generator produced what you expected, especially for interrupt handlers and inline assembly. |
| `readelf -a` (host) | Inspect ELF section layout, symbol table, and relocation entries. Essential for debugging linker script issues and verifying the kernel loaded at the right address. |
| `nm --defined-only` (host) | List all symbols defined in the kernel binary. Use this to confirm a function was compiled in, find the address of a global, or detect unexpected symbol duplication. |
| `cargo bloat` | Analyzes a Rust binary and reports which functions and crates contribute most to binary size. Useful for keeping the kernel binary lean and catching accidentally-included std dependencies. |
| `cargo expand` | Expands Rust macros in place and prints the result. Invaluable for debugging `lazy_static!`, `bitflags!`, and your own proc macros when the generated code is behaving unexpectedly. |
| `bochs` | An x86 emulator with a built-in debugger that operates at the instruction level, including real-time inspection of descriptor tables, page tables, and CPU mode. Slower than QEMU but catches hardware-level errors QEMU silently ignores. |
