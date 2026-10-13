# Tooling & Stack Preferences

- Prefers Rust (stable channel) for systems and Linux desktop applications. Confidence: 0.85
- Prefers GTK3 via gtk-rs for Linux desktop UI to match the native Cinnamon/X11 environment; explicitly rejects Electron and web-view UIs. Confidence: 0.8
- Wants the dependency set kept minimal, with every crate explicitly justified (rationale stated per dependency). Confidence: 0.85
- Prefers simple JSON files for local persistence over heavier options like SQLite, unless a database is demonstrably justified. Confidence: 0.7
- Tunes the Cargo release profile for small binaries: opt-level="z" (benchmarked against "3"), lto, codegen-units=1, panic="abort", strip=true. Confidence: 0.75
