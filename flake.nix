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

        # libgbm was split out of mesa in newer nixpkgs; fall back so this
        # works on both old and new channels.
        gbm = pkgs.libgbm or pkgs.mesa;

        runtimeLibs = with pkgs; [
          # Wayland
          wayland
          libxkbcommon
          # GPU (egui_glow path — libGL is dlopen'd at runtime)
          libGL
          libdrm
          gbm
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
          version = "0.4.1";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [ pkg-config makeWrapper ];
          buildInputs = runtimeLibs;

          # No helper PATH wrapping needed — boltsnap is fully in-process now.
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
