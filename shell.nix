# Default nix shell for Rust projects.
#
# Default: Use latest stable toolchain
# `--arg nightly`: Use nightly toolchain
# `--arg msrv`: Use MSRV toolchain
{ nightly ? false
, msrv ? false
}:
assert nightly -> !msrv;
assert msrv -> !nightly;
let
  rust-overlay = builtins.fetchTarball {
    url = "https://github.com/oxalica/rust-overlay/archive/master.tar.gz";
  };
  pkgs = (import <nixpkgs> {
    overlays = [ (import rust-overlay) ];
  });
  rust-base = if nightly
    then pkgs.rust-bin.nightly."2024-07-01".default # <-- choose your nightly
    # then pkgs.rust-bin.selectLatestNightlyWith (toolchain: toolchain.default) # <-- choose latest nightly (not recommended)
    else if msrv
      then pkgs.rust-bin.stable."1.63.0".default # <-- choose your MSRV
      else pkgs.rust-bin.stable.latest.default;
  rust = rust-base.override {
    extensions = [
      "rust-src"
      "llvm-tools-preview"
    ];
  };
in
  pkgs.mkShell.override {
    stdenv = pkgs.clang16Stdenv;
  } {
    buildInputs = [
      rust
      pkgs.just
      pkgs.gdb
      pkgs.cargo-hack
      pkgs.cargo-fuzz
      pkgs.cargo-binutils
      pkgs.rustfilt
      pkgs.pkg-config
      pkgs.openssl
    ] ++ [ # honggfuzz (legacy fuzzing)
      pkgs.libbfd
      pkgs.libunwind
    ];
    # Constants for compiler
    CC_wasm32_unknown_unknown = "${pkgs.llvmPackages_16.clang-unwrapped}/bin/clang-16";
    AR_wasm32_unknown_unknown = "${pkgs.llvmPackages_16.libllvm}/bin/llvm-ar";
    CFLAGS_wasm32_unknown_unknown = "-I ${pkgs.llvmPackages_16.libclang.lib}/lib/clang/16/include/";
    PKG_CONFIG_PATH = "${pkgs.openssl.dev}/lib/pkgconfig";

    # Constants for IDE setup
    RUST_TOOLCHAIN = "${rust}/bin";
    RUST_STDLIB = "${rust}/lib/rustlib/src/rust";
    DEBUGGER = "${pkgs.gdb}";
  }
