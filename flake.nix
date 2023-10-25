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
    systems,
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
      system,
      pkgs,
      toolchain,
      craneLib,
      ownPkgs,
      src,
      cargoArtifacts,
    }: {
      monfari = craneLib.buildPackage {
        preFixup = ''
          wrapProgram $out/bin/monfari --prefix PATH : ${pkgs.git}/bin
        '';
        nativeBuildInputs = [pkgs.makeBinaryWrapper];
        buildInputs = if system == "aarch64-darwin" then with pkgs.darwin.apple_sdk.frameworks; [ Security ] else [];
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
    apps = eachSystem ({ownPkgs}: rec {
      monfari = {
        type = "app";
        program = "${ownPkgs.monfari}/bin/monfari";
      };
      default = monfari;
    });
    nixosModules.monfari = { lib, pkgs, config, ... }: with lib; let
      cfg = config.services.bluepython508.monfari;
    in {
      options.services.bluepython508.monfari = {
        enable = mkEnableOption "monfari server";
        address = mkOption {
          type = types.str;
        };
      };
      config.systemd = mkIf cfg.enable { 
        services.monfari = {
          description = "Monfari";
          environment = {
            MONFARI_REPO = "path:/var/lib/monfari";
            RUST_BACKTRACE = "1";
            RUST_SPANTRACE = "1";
            RUST_LOG = "debug";
            GIT_AUTHOR_NAME = "User";
            GIT_AUTHOR_EMAIL = "user@example.org";
            GIT_COMMITTER_NAME = "User";
            GIT_COMMITTER_EMAIL = "user@example.org";
          };
          wantedBy = [ "multi-user.target" ];
          serviceConfig = {
            ExecStartPre = ["-${self.packages.${pkgs.system}.monfari}/bin/monfari init /var/lib/monfari"];
            ExecStart = "${self.packages.${pkgs.system}.monfari}/bin/monfari serve http ${cfg.address}";
            ExecStop = "${lib.getExe pkgs.curl} -XPOST http://${cfg.address}/__stop__";
            DynamicUser = true;
            ProtectHome = true;
            PrivateUsers = true;
            StateDirectory = "monfari";
          };
        };
      };
    };
  };
}
