# BASR branding resources

These SVGs are imported byte-for-byte from the user-designated
**Ferris · 八接头版 v4** resources in the supplied `ferris-logo/v4` directory.
The GUI uses the regular outline variants, not the simplified `-32` variants.
No preview images, raster exports, or generator code are included.

| File | Use |
| --- | --- |
| `ferris-outline.svg` | About-window logo on light backgrounds |
| `ferris-outline-light.svg` | About-window logo on dark backgrounds |
| `ferris-dimensional.svg` | Default scalable desktop launcher icon |
| `ferris-dimensional-32.svg` | Optical 32px desktop launcher icon |

The regular outline preserves the eight three-part connector collars and curved
closed eyes. The simplified `-32` outline has different details and is not used.
The outlines retain transparent interiors and vector masks; the dimensional
versions retain their original colors and gradients.

## GUI display size

The main header uses a menu icon instead of a small logo. The original outline
appears at a larger size in About. Only the element's display size changes;
the source SVG's geometry, stroke weights and colors remain untouched.

## Desktop lookup

The Nix package installs the dimensional SVGs in hicolor and writes an absolute
`Icon` path into its desktop entry. This avoids relying on a launcher's cached
theme-name lookup for icons newly added to a Nix profile.

## Provenance and licensing

The user supplied and requested these design resources for BASR. The supplied
resource README does not state a separate artwork license. Importing the files
does not assert third-party ownership or grant a third-party license, nor does
it relabel the artwork as MIT. BASR's first-party code remains MIT.

## Original SHA-256 checksums

```text
e2a554a564af6770b6954d58c07a9b0c753d4291428a43cb613202723e397e2a  ferris-outline.svg
fa313b6234ededa1131e5a503524d109392ed3939157d7dfb501f9a7607395fd  ferris-outline-light.svg
c42c4cb2caa2fbf02a1205dd305096fea1c49b5e2fa13b2fa60861ea6bc4ac69  ferris-dimensional.svg
b5f6163aea4e46e60d14f76a52d79ea134c80ff3ba523f4be4387b1d784e2f8a  ferris-dimensional-32.svg
```
