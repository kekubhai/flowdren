# Flowdren — when to run which commands

Run everything from the **repo root**: `D:\Web3-PrJKts\flowdren`  
(Not from `programs/` or `programs/flowdren/`.)

This is an **Anchor program**, not a Rust binary. Do **not** use `cargo run`.

---

## Day-to-day (most of the time)

| When | Command |
|------|---------|
| Build the on-chain program only | `anchor build` |
| Build + start local validator + run tests | `anchor test` |
| Same as above (npm) | `npm test` |
| Install/update JS test deps | `npm install` |

```powershell
cd D:\Web3-PrJKts\flowdren
npm install          # once, or after package.json changes
anchor build         # compile the program
anchor test          # full local test run
```

---

## First-time / new machine

1. Install **Rust**, **Solana CLI** (Agave), **Anchor** (`avm` / `anchor-cli` **0.31.1**), **Node.js**.
2. Confirm versions:

```powershell
solana --version      # e.g. 2.2.x
anchor --version      # 0.31.1
node --version
rustc --version
```

3. Then from repo root:

```powershell
npm install
anchor build
anchor test
```

---

## If `anchor build` / `anchor test` fails

### A. “Solana toolchain is corrupted”

`--force-tools-install` alone often **reuses** a broken cache. Wipe, then reinstall.

```powershell
# 1) Remove broken cache + unlink
rustup toolchain uninstall solana
Remove-Item -Recurse -Force "$env:USERPROFILE\.cache\solana\v1.47" -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force "C:\solana\bin\platform-tools-sdk\sbf\dependencies\platform-tools" -ErrorAction SilentlyContinue

# 2) Download + extract platform-tools (v1.47 matches this Solana CLI)
New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.cache\solana\v1.47\platform-tools" | Out-Null
curl.exe -L -o "$env:USERPROFILE\.cache\solana\v1.47\platform-tools-windows-x86_64.tar.bz2" `
  https://github.com/anza-xyz/platform-tools/releases/download/v1.47/platform-tools-windows-x86_64.tar.bz2
tar -xjf "$env:USERPROFILE\.cache\solana\v1.47\platform-tools-windows-x86_64.tar.bz2" `
  -C "$env:USERPROFILE\.cache\solana\v1.47\platform-tools" --strip-components 1
Remove-Item "$env:USERPROFILE\.cache\solana\v1.47\platform-tools-windows-x86_64.tar.bz2"

# 3) Link into Solana SDK
cmd /c mklink /J "C:\solana\bin\platform-tools-sdk\sbf\dependencies\platform-tools" `
  "$env:USERPROFILE\.cache\solana\v1.47\platform-tools"
New-Item -ItemType File -Force -Path "C:\solana\bin\platform-tools-sdk\sbf\dependencies\platform-tools-v1.47.md" | Out-Null

# 4) Windows 2.2.14 workaround: cargo-build-sbf looks for rustc/cargo WITHOUT .exe
$bin = "$env:USERPROFILE\.cache\solana\v1.47\platform-tools\rust\bin"
Copy-Item "$bin\rustc.exe" "$bin\rustc" -Force
Copy-Item "$bin\cargo.exe" "$bin\cargo" -Force
rustup toolchain link solana "$env:USERPROFILE\.cache\solana\v1.47\platform-tools\rust"

# 5) Rebuild
cargo-build-sbf
# or
anchor build
```

Need ~3 GB free on `C:` for download + extract.

### B. `feature edition2024 is required`

Solana’s bundled Cargo is **1.84** and cannot parse some newer crates. This repo already has:

- `.cargo/config.toml` — MSRV fallback resolver  
- pinned versions in `Cargo.lock`

If it comes back after a blind `cargo update`:

```powershell
cargo generate-lockfile
cargo update -p blake3 --precise 1.8.2
cargo update -p constant_time_eq --precise 0.3.1
cargo update -p indexmap --precise 2.11.4
# pin any other crate named in the error to the previous version
anchor build
```

### C. `cargo run` → “a bin target must be available”

Wrong command. Use `anchor build` / `anchor test` from the **repo root**.

---

## Optional / lower-level commands

| When | Command |
|------|---------|
| Build BPF/SBF without Anchor wrapper | `cargo-build-sbf` |
| Force platform-tools reinstall (only after wiping bad cache) | `cargo-build-sbf --force-tools-install` |
| Check Solana keypair exists | `solana-keygen new` (if missing `~/.config/solana/id.json`) |
| Local cluster only (no tests) | `solana-test-validator` |

---

## Quick decision tree

```
Need to compile the program?     →  anchor build
Need to run tests?               →  anchor test
Changed package.json?            →  npm install
Toolchain corrupted?             →  wipe cache + manual install (section A)
edition2024 error?               →  re-pin crates (section B)
Tried cargo run?                 →  stop; use anchor instead
```
