# Source dependencies

## gds-uim

- Upstream: https://github.com/4t145/gds-uim
- Integration: unmodified Git submodule at `vendor/gds-uim`; the Git link pins its commit.
- Current revision: `e955e9bdd828fcb4931dc9dade60f8470f21db8f`.
- Usage: optional GUI-only Cargo path dependency providing `el!` and `m!`.
  The library has no runtime dependencies; its own example/test GPUI dependency
  is not part of Bas's dependency graph.
- License: this revision has no license declaration or license file. It is
  integrated at the author's request, not relicensed under Bas's MIT license.
  Upstream licensing must be clarified before redistribution.

Initialize the pinned dependency after cloning or pulling:

```sh
git submodule update --init --recursive
```

Cargo and `nix develop .#gui` read this initialized physical checkout. Prefer
`make install-bas` / `make install-basg` for local installation: their path
snapshots include untracked source and initialized submodule source, excluding
`.git`, `.delta`, `.env`, and other metadata. Path sources need no Git fetching
options. Direct Git package commands must request submodules explicitly, for
example `nix build "git+file://$PWD?submodules=1#basg"`. Remote flake inputs use
`git+https://github.com/4t145/bas?submodules=1`, not the `github:` tarball fetcher.
See the [Nix source boundary](../docs/upstream-ui.md#nix-source-类型边界).

Do not copy or patch the macro implementation into Bas. Changes belong upstream;
update the submodule's pinned revision separately from application rendering changes.

The current upstream revision reduces nested expansion overhead while preserving
temporary borrows. Rust child expressions containing braces or indexing use the
explicit `$(...)` boundary; Bas's settings view uses `$(match ...)`.
