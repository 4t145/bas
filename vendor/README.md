# Source dependencies

This directory contains documentation only, not vendored dependency source.

## gds-uim

- Upstream: https://github.com/4t145/gds-uim
- Integration: unmodified Cargo Git dependency, as recommended by the upstream README.
  Bas pins a full `rev` in `Cargo.toml`; `Cargo.lock` records both workspace packages.
- Current revision: [`94fe061888c45e137daff0d45c172c36bdb09882`](https://github.com/4t145/gds-uim/tree/94fe061888c45e137daff0d45c172c36bdb09882).
- Usage: optional GUI-only dependency providing `el!` and `m!` by re-exporting
  the `gds-uim-macros` procedural macro crate. Its `syn`, `quote`, and `proc-macro2`
  dependencies are compile-time only. Upstream's GPUI example/test dev-dependency
  is not part of Bas's runtime dependency graph.
- License: neither package manifest declares `license` or `license-file`, and
  this revision's repository tree has no license file. The Git integration
  recommendation is not a license grant; Bas's MIT license does not cover this
  dependency. Upstream authorization must be clarified before redistribution.

No submodule initialization is needed. Cargo fetches the locked Git source;
Nix's `cargoLock.outputHashes` in `nix/package.nix` verifies that same checkout.
The two packages share one Git source hash:

```sh
nix flake prefetch --json \
  'git+https://github.com/4t145/gds-uim.git?rev=94fe061888c45e137daff0d45c172c36bdb09882'
```

The measured NAR hash is `sha256-yVojxpvvejy2NBKgcKsffBMqrPqe6YOJ94vtjCUz7dQ=`.
Update the revision, lockfile and hash together, and verify the actual Nix
dependency fetch before accepting a new revision. No dependency source is
copied into the installation snapshot. See the
[Nix source boundary](../docs/upstream-ui.md#nix-source-类型边界).

Do not copy or patch the macro implementation into Bas. Changes belong upstream;
never modify the local Cargo cache to work around a parser or rendering issue.

The current [grammar](https://github.com/4t145/gds-uim/blob/94fe061888c45e137daff0d45c172c36bdb09882/docs/grammer.md)
uses postfix function calls (`.div`, `"id".Button::new`), property closures
(`hover: { ... }`) and `@if` / `@match` control blocks. Bare identifiers refer to
existing values, not implicit constructors. Use `$(...)` for Rust expressions
that need UIM suffixes; see the [upgrade notes](../docs/upstream-ui.md#git-依赖与新语法升级).
This revision fixes [upstream #1](https://github.com/4t145/gds-uim/issues/1), so
optional children can use `@if (let Some(value) = option)` without a fallback match.
