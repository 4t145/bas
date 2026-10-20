# About build identity

About displays the package version from Cargo (no separate version constant) and
a compile-time build tag. Tags contain no timestamps, usernames, paths, or remote
URLs by default.

- **Nix:** `nix-source-<32-character source store hash>`. This identifies the
  filtered source input in `nix/package.nix`, including uncommitted files included
  in the snapshot. It works with `scripts/nix-source` and other sources without
  `.git`. Identical source inputs produce the same tag; it is not a Git revision,
  binary hash, or identity of the compiler/dependency environment.
- **Direct Cargo:** `git-<12-character HEAD>-worktree`. HEAD is only the base
  revision. The suffix always appears, even in clean checkouts: working-tree
  changes are **not** fingerprinted or asserted absent. Multiple local builds
  from the same HEAD can therefore share a tag. Git HEAD, refs, and packed refs
  are watched, including Git worktree metadata outside the source directory.
- **No usable Git revision:** `source-unknown`, explicitly marking missing
  provenance rather than implying a release or clean checkout.

CI/release builds can set `BAS_BUILD_TAG=release-0.1.0+ci.42 cargo build ...`.
Changing/removing the variable invalidates Cargo's build metadata. Tags must
contain 1–80 ASCII letters, digits, `.`, `_`, `-`, or `+`; invalid overrides fail
the build rather than silently falling back. Override values are displayed
verbatim, so use public identifiers, never secrets. In Nix, pass the optional
`buildTag` package argument (for example through `.override`); otherwise Nix
supplies the source tag automatically.

Metadata unit tests run with `cargo test --test build_metadata`. The GUI
`application::tests::about_renders_explicit_version_and_build_tag` test checks
the rendered labeled fields with `gui-tests` enabled.
