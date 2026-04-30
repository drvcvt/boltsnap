{
  description = "boltsnap — fast in-process Wayland + X11 screenshot tool with built-in editor";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        lib = pkgs.lib;

        runtimeLibs = with pkgs; [
          # Wayland
          wayland
          libxkbcommon
          # GPU / wgpu (vulkan-loader is dlopen'd, must be in LD path)
          vulkan-loader
          libGL
          libdrm
          # X11 stack (winit links it even on Wayland)
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
          xorg.libxcb
          # Fonts
          fontconfig
          freetype
        ];

        rpath = lib.makeLibraryPath runtimeLibs;
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "boltsnap";
          version = "0.3.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = runtimeLibs;

          postFixup = ''
            patchelf --set-rpath "${rpath}" $out/bin/boltsnap
            wrapProgram $out/bin/boltsnap \
              --prefix LD_LIBRARY_PATH : ${rpath}
          '';

          meta = with lib; {
            description = "Fast in-process Wayland + X11 screenshot tool with built-in annotation editor";
            homepage = "https://github.com/drvcvt/boltsnap";
            license = licenses.mit;
            platforms = platforms.linux;
            mainProgram = "boltsnap";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [
            cargo
            rustc
            rust-analyzer
            pkg-config
          ]) ++ runtimeLibs;

          LD_LIBRARY_PATH = rpath;
        };
      });
}
