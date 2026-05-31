# my-kernel

A bare-metal x86_64 kernel written in Rust (no_std), booted via the Limine bootloader. The goal is a working terminal shell, built phase-by-phase through memory management, interrupt handling, input drivers, a TTY layer, cooperative tasking, and finally a command shell. See `plan.md` for the full roadmap.

---

## Language conventions

- Use American English spellings throughout: `color` not `colour`, `initialize` not `initialise`, `behavior` not `behaviour`, `center` not `centre`, `gray` not `grey`, etc.

---

## Text encoding for code

Always use plain ASCII punctuation in code, inline code spans, and code blocks:

| Use | Never use |
|-----|-----------|
| Straight double quote `"` | Curly quotes `"` `"` |
| Straight single quote `'` | Curly quotes `'` `'` |
| ASCII backtick `` ` `` | Unicode lookalikes |
| Three periods `...` | Ellipsis character `...` (U+2026) |
| Hyphen-minus `-` | En dash `-` (U+2013) or em dash `--` (U+2014) |
| ASCII apostrophe `'` | Right single quotation mark `'` (U+2019) |
| ASCII angle brackets `<` `>` | `<<` `>>` `<<` `>>` |

This applies in all output: comments, doc strings, prose explanations, and markdown.

---

## Build and run

```
make               # build the kernel image
./run.sh           # launch in QEMU
```

Direct cargo invocation (if needed):

```
cargo build --target x86_64-crusty_os.json -Z build-std=core,compiler_builtins -Z build-std-features=compiler-builtins-mem
```
