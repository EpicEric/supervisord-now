{
  system ? builtins.currentSystem,
  inputs ? import ./.tack,
  pkgs ? import inputs.nixpkgs { inherit system; },
}:
pkgs.mkShell {
  packages = [
    pkgs.cargo
    pkgs.clippy
    pkgs.nil
    pkgs.nixfmt-rs
    pkgs.nodejs_24
    pkgs.oxfmt
    pkgs.oxlint
    pkgs.rustc
  ];
}
