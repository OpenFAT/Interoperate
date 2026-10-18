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

      # The Schemify commit Cargo.toml pins (the `rev` of `schemify-abi`), for `devices/`:
      # schemify-doc embeds `../../../devices/*.shprim` from outside its crate directory, which
      # vendoring (one directory per crate) leaves behind.
      schemifyRev =
        let
          line =
            nixpkgs.lib.findFirst (nixpkgs.lib.hasPrefix "schemify-abi = ")
              (throw "no schemify-abi in Cargo.toml")
              (nixpkgs.lib.splitString "\n" (builtins.readFile ./Cargo.toml));
        in
        builtins.head (builtins.match ''.*rev = "([0-9a-f]+)".*'' line);
      schemifySrc = builtins.fetchGit {
        url = "https://github.com/OmarSiwy/Schemify";
        rev = schemifyRev;
      };

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
          # The vendored crates sit in $NIX_BUILD_TOP/cargo-vendor-dir, so `devices/` goes beside it.
          preBuild = ''
            cp -r ${schemifySrc}/devices "$NIX_BUILD_TOP/devices"
          '';
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
