# Contributing

Thanks for your interest in cliprust. This is a small, deliberately dependency-light
project; please keep changes focused.

## Ground rules

- **All changes go through a pull request to `main`.** Direct pushes are blocked.
- **Every PR needs an approving review from the maintainer (@Alvee3120)** before
  it can be merged. This is enforced with `CODEOWNERS` plus branch protection
  (see `.github/CODEOWNERS` and `scripts/protect-branch.sh`).
- CI must be green: `fmt`, `clippy -D warnings`, unit tests, and the smoke test.
- Keep the dependency set minimal. If you add a crate, justify it in
  `Cargo.toml` the way the existing ones are; the project prefers pure-Rust,
  no-C-build crates.

## Develop

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Unit tests are `#[cfg(test)]` modules next to the code (storage, config, image,
URL detection). They must run **headless** — no display, no X server.

## End-to-end smoke test

```sh
./scripts/smoke-test.sh
```

It starts the real daemon against a private temp tree (own `XDG_*` dirs), drives
the clipboard with `xclip`, and asserts on the JSON history and image sidecars.
It needs a display but no window manager; with no `$DISPLAY` it re-execs itself
under `xvfb-run`, so it works unchanged in CI.

## A note on file modes

If you clone onto a filesystem that cannot store the Unix execute bit (NTFS, most
external drives), git reports `core.fileMode=false` and new `*.sh` files are
committed as `100644`. Mark them executable in the index explicitly:

```sh
git update-index --chmod=+x scripts/smoke-test.sh install.sh
```

CI and the docs invoke the scripts via `bash …` where possible, but `install.sh`
and `scripts/*.sh` are meant to be runnable directly.

## Releases (maintainers)

1. Bump `version` in `Cargo.toml` and commit.
2. Tag and push: `git tag v0.1.1 && git push origin v0.1.1`.
3. The `Release` workflow builds the `.deb` with `cargo-deb` and attaches it to
   the GitHub Release.

## First-time repo setup (maintainer)

After the repo exists, run once to lock `main` down:

```sh
scripts/protect-branch.sh            # requires the gh CLI, authenticated as admin
```

This requires an approving code-owner review, dismisses stale approvals, and
requires the `check` and `smoke` CI jobs to pass before a PR can merge.
