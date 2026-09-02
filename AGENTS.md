# Repository Guidelines

## Project Structure & Module Organization

MemRW3 is a single Rust 2024 desktop binary. `src/main.rs` starts the eframe application, while `src/app.rs` coordinates UI state, configuration, probe access, and the acquisition thread. Keep domain code in its existing modules: `src/dwarf/` parses ELF/DWARF data, `src/model/` owns session and buffering types, `src/probe/` wraps `probe-rs`, and `src/ui/` contains egui views plus the Chart and Table plugins. Consult `ARCHITECTURE.md` before changing data flow or plugin boundaries; `README.md` documents setup and user workflows. Build output belongs in `target/` and must not be committed.

## Build, Test, and Development Commands

- `cargo build` — compile a debug build for quick feedback.
- `cargo run --release` — launch the optimized GUI locally.
- `cargo build --release --verbose` — reproduce the CI build; the binary is `target/release/MemRW3` (`.exe` on Windows).
- `cargo test --release --verbose` — run the same test command used by CI.
- `cargo fmt --all` — apply standard Rust formatting; use `cargo fmt --all -- --check` for a non-mutating check.

Use a stable toolchain supporting edition 2024 (Rust 1.85 or newer). Linux system packages are listed in `README.md`.

## Coding Style & Naming Conventions

Follow rustfmt defaults and four-space indentation. Name files, modules, functions, and variables in `snake_case`; types and traits in `PascalCase`; constants in `SCREAMING_SNAKE_CASE`. Keep UI concerns under `src/ui/` and hardware side effects in the application/probe layers. Plugins should communicate outward through `PluginAction`. Every `unsafe` block or implementation must retain a precise `// SAFETY:` justification, especially around acquisition buffers and cached probe cores.

## Testing Guidelines

The repository currently has no dedicated test suite or coverage threshold. Add focused unit tests beside implementation code in `#[cfg(test)] mod tests`; place cross-module scenarios in `tests/<feature>.rs`. Use descriptive names such as `decodes_signed_16_bit_value`. Prefer deterministic tests that do not require a physical probe. For hardware or UI changes, record manual test steps, target MCU/probe, and observed results in the pull request.

## Commit & Pull Request Guidelines

Recent history favors short imperative subjects, sometimes with prefixes such as `feat:`, `fix:`, or `clean:`. Keep commits focused; for example, `fix: resolve inherited DWARF fields`. Target pull requests to `master`, explain behavior and risks, link relevant issues, and include screenshots for visual changes. Confirm the Ubuntu and Windows release build/test workflow passes. Do not commit firmware ELF/AXF files, local JSON configurations, CSV logs, device identifiers, or machine-specific paths.
