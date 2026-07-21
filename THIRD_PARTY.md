# Third-party notices

The Spectrum creative suite is MIT licensed. Its dependency graph is recorded
exactly in `Cargo.lock`.

Sony ARW decoding and development uses `rawler` 0.7.2, Copyright (c) the
dnglab/rawler contributors, under the GNU Lesser General Public License v2.1.
Source and license text: <https://github.com/dnglab/dnglab/tree/main/rawler>.

Prism uses the Ubuntu Light font distributed by `epaint_default_fonts` for
portable text-layer rendering. Ubuntu Font Family is licensed under the Ubuntu
Font Licence 1.0: <https://ubuntu.com/legal/font-licence>.

The optional, standalone Prism terminal proof harness builds and statically
links Ghostty 1.3.1, Copyright (c) 2024 Mitchell Hashimoto, under the MIT
License. The exact official source archive checksum, annotated tag object,
peeled source revision, toolchain, and generated artifact contract are recorded
in `packaging/prism/macos/ghostty-proof.lock`. The proof bundle includes
Ghostty's license as `Contents/Resources/GHOSTTY-LICENSE`. Ghostty is not linked
into the production Prism application by this proof.

Packaged builds include this notice. Anyone distributing the suite should review
the LGPL requirements for their distribution model and retain a relinkable or
otherwise compliant form of the application.
