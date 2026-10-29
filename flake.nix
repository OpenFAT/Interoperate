{
  description = "Schemify interop plugin: xschem and Virtuoso import/export, as a wasm guest and the native CLI schemify-interop";

  inputs = {
    # Locked to the nixpkgs and rust-overlay revisions Schemify's flake.lock pins: Rust 1.95.0
    # with rust-lld builds the guest byte for byte (build.sh).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );

      # Rust 1.95.0 (the rust-version) with the guest and WASI targets and rust-lld.
      toolchain =
        pkgs:
        pkgs.rust-bin.stable."1.95.0".default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
          targets = [
            "wasm32-unknown-unknown"
            "wasm32-wasip1"
          ];
        };
      rustPlatform =
        pkgs:
        pkgs.makeRustPlatform {
          cargo = toolchain pkgs;
          rustc = toolchain pkgs;
        };

      # The native CLI (`schemify-interop xschem-import …`).
      interop =
        pkgs:
        (rustPlatform pkgs).buildRustPackage {
          pname = "schemify-interop";
          version = "0.1.0";
          src = self;
          cargoLock = {
            lockFile = ./Cargo.lock;
            # The Schemify crates are git dependencies pinned to one rev in Cargo.toml.
            allowBuiltinFetchGit = true;
          };
          cargoBuildFlags = [
            "--features"
            "cli"
            "--bin"
            "schemify-interop"
          ];
          # The suite needs the fetched corpus and xschem; run `cargo test --features cli` in
          # the dev shell (CI does).
          doCheck = false;
          meta = {
            description = "Schemify interop plugin, native CLI (xschem import)";
            homepage = "https://github.com/OpenFAT/Interoperate";
            license = pkgs.lib.licenses.mit;
            mainProgram = "schemify-interop";
            platforms = systems;
          };
        };
    in
    {
      packages = forAll (pkgs: {
        default = interop pkgs;
        schemify-interop = interop pkgs;
      });

      apps = forAll (pkgs: {
        default = {
          type = "app";
          program = "${interop pkgs}/bin/schemify-interop";
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            (toolchain pkgs)
            # tests/xschem_export.rs netlists the export with a real xschem; the corpus
            # (corpus/golden.sh) makes goldens with it, tclsh for the PDK generators.
            xschem
            tcl
            # The differential's second oracle (ngspice results) and the WASI determinism run.
            ngspice
            wasmtime
            nixfmt
          ];
        };
      });

      formatter = forAll (pkgs: pkgs.nixfmt);
    };
}
