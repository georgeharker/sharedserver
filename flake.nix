{
  description = "A shared process manager with reference counting, grace periods, and dead-client detection";

  inputs = {
    crane.url = "github:ipetkov/crane";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-parts = {
      url = "github:hercules-ci/flake-parts";
      inputs.nixpkgs-lib.follows = "nixpkgs";
    };
  };

  outputs = inputs:
    inputs.flake-parts.lib.mkFlake {inherit inputs;} {
      systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
      imports = [inputs.flake-parts.flakeModules.easyOverlay];
      perSystem = {
        lib,
        config,
        pkgs,
        ...
      }: let
        cargoToml = builtins.fromTOML (builtins.readFile ./rust/Cargo.toml);
        inherit (cargoToml.package) name description repository;
        meta = {
          inherit description;
          mainProgram = name;
          homepage = repository;
          license = lib.licenses.mit;
        };
        craneLib = inputs.crane.mkLib pkgs;
        sourceFilter = path: type: (builtins.match ".*/test_helpers/.*\\.sh$" path != null) || (craneLib.filterCargoSources path type);
        cargoArtifacts = craneLib.buildDepsOnly {
          src = ./rust;
          strictDeps = true;
        };
        sharedserver = craneLib.buildPackage {
          inherit cargoArtifacts meta;
          src = lib.cleanSourceWith {
            src = ./.;
            filter = sourceFilter;
          };
          cargoToml = rust/Cargo.toml;
          cargoLock = rust/Cargo.lock;
          postUnpack = ''
            cd $sourceRoot/rust
            sourceRoot="."
          '';
          # Isolate tests from the host's live lockdir (the Nix sandbox is off
          # by default on Darwin, so tests would otherwise hit /tmp/sharedserver)
          preCheck = ''
            export SHAREDSERVER_LOCKDIR="$TMPDIR/sharedserver-tests"
          '';
        };
        sharedserver-nvim = pkgs.vimUtils.buildVimPlugin {
          pname = "sharedserver-nvim";
          version = toString (inputs.self.shortRev or inputs.self.dirtyShortRev or inputs.self.lastModified or "git");
          src = with lib.fileset;
            toSource {
              root = ./.;
              fileset = unions (map maybeMissing [./lua ./plugin ./ftdetect ./doc]);
            };
        };
      in {
        packages = {
          inherit sharedserver sharedserver-nvim;
          default = sharedserver;
        };
        overlayAttrs = config.packages;
      };
    };
}
