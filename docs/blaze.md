# Build with blaze

[blaze](https://github.com/indexable-inc/blaze) builds a Rust program without rustc. A daemon keeps the workspace compiled and splices changed functions into the program. This document records the pin, the checks of the repository, and the measurements that keep blaze out of the merge gate.

## The pin

`.github/blaze-version` names the blaze release. `scripts/blaze.sh` installs that release when the machine has another one, then runs the blaze command of its arguments. `mise run blaze-check` runs `check --workspace --all-targets` through it.

blaze builds every project with its own Rust toolchain, whatever `mise.toml` pins. Release 0.20261027.0 builds with `nightly-2026-10-27`. The project keeps Rust 1.99.0, and `mise run check` stays the final word.

## The CI job

`.github/workflows/blaze.yml` installs the pinned release, restores the release, its toolchain, and its store from the Actions cache, and runs the same check. The job turns off the AppArmor restriction of Ubuntu on user namespaces, because blaze runs each build script in a sandbox that it makes. The job sets `continue-on-error: true`: it reports what blaze says and does not block a merge. A pull request keeps one required status, `checks`.

The job does not use blaze's remote store, because that needs a cache service and a token that this repository does not have. Each run warms its own store from the Actions cache.

The first run on Linux x86-64 (2026-10-30, run 40978160026) installed the release in 10 s and downloaded its 159 MB toolchain in 6 s. A warm run (run 40979692359) restored the release, the toolchain, and the store from a 317 MB cache entry and reached the failure in 26 s. The failing step keeps the job green, so the job saves its cache, and a later step adds a warning that names this document. The failure is the one below.

## The blocker, measured 2026-10-30

`blaze check --workspace --all-targets` cannot load this workspace, on macOS arm64 and on Linux x86-64. The output ends with:

```text
blaze could not compile the build script of `libz-sys v1.1.30` for the load; blaze did not build it (exit status: 1): Error: `build_script_build` reaches 1 function blaze could not compile, and a build with nothing of rustc's has nothing else to compile it, so it is not built:;   <cc::parallel::job_token::inherited_jobserver::HelperThread>::new: unsupported: callee `into_helper_thread`: callee `{closure}`: callee `send`: callee `send`: callee `send`: callee `with`: callee `{closure}::fn_pointer`: callee `{closure}`: unsupported: writable static `__RUST_STD_INTERNAL_VAL` without a rustc symbol
```

The build scripts of `libz-sys`, `zstd-sys`, and `openssl-sys` compile C with `cc`. Each checkout reaches a different one of them first. The jobserver path of `cc` starts a helper thread, and its channel send reads a writable static of `std` that rustc exports under no symbol. blaze refuses that class on purpose: a second copy of the static would not be the copy that `std` writes (`docs/no-rustc.md`, step E of blaze issue #85, and `crates/hir-ty-codegen/mod.rs`). The same class fails other projects that build C code, such as `aws-lc-sys`, `bat`, and `biome` in the benchmark report of the release.

Three changes gave the same failure, and none of them is in the repository:

- `cargo build --workspace --all-targets` before the check. The load then reused cargo's build script of `libz-sys` and failed on the build script of `zstd-sys` in the same way.
- `cargo update -p cc --precise 1.2.41`. Release 1.2.41 of `cc` has the same jobserver helper thread.
- `[env] CARGO_MAKEFLAGS = { value = "", force = true }` in `.cargo/config.toml`. blaze gives a build script its own jobserver, so the setting does not reach the script.

## What makes the job a gate

A blaze release that loads this workspace. `blaze build --bin pbeam` and `blaze test` must then agree with cargo on the same tree. Then the job drops `continue-on-error` and joins `mise run check`. Until then the job records the progress of that support on each pull request.
