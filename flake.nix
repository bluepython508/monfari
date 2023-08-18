{
  description = "Accounting";

  inputs = {
    nixpkgs.url = "flake:nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    treefmt-nix,
    crane,
    systems
  }: let
    inherit (nixpkgs) lib;
    eachSystem = f:
      lib.genAttrs (import systems) (system:
        f (lib.getAttrs (lib.attrNames (lib.functionArgs f)) rec {
          inherit system;
          pkgs = import nixpkgs {
            inherit system;
            overlays = [rust-overlay.overlays.default];
          };
          ownPkgs = self.packages.${system};
          toolchain = pkgs.rust-bin.fromRustupToolchainFile ./toolchain.toml;
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
          src = craneLib.cleanCargoSource (craneLib.path ./.);
          cargoArtifacts = craneLib.buildDepsOnly {
            inherit src;
          };
          treefmt =
            (treefmt-nix.lib.evalModule pkgs {
              projectRootFile = "flake.nix";
              programs = {
                alejandra.enable = true;
                rustfmt = {
                  enable = true;
                  package = toolchain;
                };
              };
            })
            .config
            .build;
        }));
  in {
    packages = eachSystem ({
      pkgs,
      toolchain,
      craneLib,
      ownPkgs,
      src,
      cargoArtifacts,
    }: {
      monfari = craneLib.buildPackage {
        inherit src cargoArtifacts;
      };
      default = ownPkgs.monfari;
    });
    devShells = eachSystem ({
      pkgs,
      toolchain,
      ownPkgs,
    }: {
      default = pkgs.mkShell {
        inputsFrom = [ownPkgs.default];
        packages = [toolchain pkgs.bacon];
      };
    });
    formatter = eachSystem ({treefmt}: treefmt.wrapper);
    checks = eachSystem ({
      craneLib,
      ownPkgs,
      src,
      cargoArtifacts,
      treefmt,
    }: {
      inherit (ownPkgs) monfari;
      formatting = treefmt.check self;
      clippy = craneLib.cargoClippy {
        inherit src cargoArtifacts;
      };
    });
  };
}
