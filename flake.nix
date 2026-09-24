{
  description = "Spectrum Rust and native Linux development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

  outputs = { nixpkgs, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
    in {
      devShells = nixpkgs.lib.genAttrs systems (system:
        let
          pkgs = import nixpkgs { inherit system; };
          runtimeLibraries = with pkgs; [
            libGL
            libxkbcommon
            wayland
            libX11
            libXcursor
            libXi
            libXrandr
            libxcb
          ];
        in {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo rustc rustfmt clippy rust-analyzer
              pkg-config cmake
              rustPlatform.bindgenHook
            ];
            buildInputs = runtimeLibraries;
            # Winit/glutin load these libraries dynamically. Prefer the host's
            # graphics drivers, which must match its running NixOS generation.
            LD_LIBRARY_PATH = "/run/opengl-driver/lib:${pkgs.lib.makeLibraryPath runtimeLibraries}";
            RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
            # Keep test fixtures on the workspace mount: separate /tmp bind
            # mounts can reject revision-cache inode exchanges with EXDEV.
            shellHook = ''
              export TMPDIR="$PWD/target/tmp"
              mkdir -p "$TMPDIR"
            '';
          };
        });
    };
}
