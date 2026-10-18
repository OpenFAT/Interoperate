{
  description = "Schemify interop plugin: xschem and Virtuoso import/export, as a wasm guest and the native CLI schemify-interop";

  inputs = {
    # Locked to the nixpkgs revision Schemify's flake.lock pins: the same rustc builds the guest
    # byte for byte (build.sh).
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # The native CLI (`schemify-interop xschem-import …`).
      interop =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
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
            homepage = "https://github.com/OmarSiwy/Schemify-Interop";
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
            # Rust (nixpkgs' rustc ships the wasm32-unknown-unknown std); wasm-ld, since the
            # nix rustc has no rust-lld (build.sh).
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            lld
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
