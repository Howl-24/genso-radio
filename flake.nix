{
  description = "Rust template using Naersk and Fenix";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix.url = "github:nix-community/fenix";
    fenix.inputs.nixpkgs.follows = "nixpkgs";
    naersk.url = "github:nix-community/naersk";
  };

  outputs = { self, nixpkgs, fenix, naersk, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = (import nixpkgs) {
          inherit system;
        };

        toolchain = with fenix.packages.${system}; combine [
          minimal.cargo
          minimal.rustc
        ];

        naersk' = naersk.lib.${system}.override {
          cargo = toolchain;
          rustc = toolchain;
        };
      in rec {
        defaultPackage = packages.${system};

        packages.${system} = naersk'.buildPackage {
          src = ./.;
          strctDeps = true;

          buildInputs = with pkgs; [
            alsa-lib
            openssl
          ];

          nativeBuildInputs = with pkgs; [
            pkg-config
            makeWrapper
          ];

          postInstall = ''
            mkdir -p $out/lib
            ln -s ${pkgs.alsa-lib}/lib/libasound.so.2 $out/lib
          '';
        };

        doCheck = true;

        # Devshell to run cargo builds
        devShells.default = pkgs.mkShell rec {
          # Include libraries in buildInputs
          buildInputs = with pkgs; [
            rust-analyzer
            toolchain
            pkg-config
            alsa-lib
            openssl
          ];

          LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath buildInputs}";

          shellHook = ''
            echo Rust!
          '';
        };
      });
}
