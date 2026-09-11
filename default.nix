{
  system ? builtins.currentSystem,
  inputs ? import ./.tack,
  pkgs ? import inputs.nixpkgs { inherit system; },
}:
pkgs.callPackage ./package.nix {
  now = import inputs.now { inherit pkgs; };
  supervisord = pkgs.callPackage ./nix/supervisord/package.nix { };
  supervisord-now = pkgs.callPackage ./rust/package.nix {
    web = pkgs.callPackage ./web/package.nix { };
  };
  inherit inputs;
}
