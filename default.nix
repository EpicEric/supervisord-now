{
  system ? builtins.currentSystem,
  inputs ? import ./.tack,
  pkgs ? import inputs.nixpkgs { inherit system; },
}:
pkgs.callPackage ./package.nix {
  now = import inputs.now { inherit pkgs; };
  supervisord = pkgs.callPackage ./nix/supervisord/package.nix { };
}
