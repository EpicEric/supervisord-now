{
  system ? builtins.currentSystem,
  inputs ? import ./.tack,
  pkgs ? import inputs.nixpkgs { inherit system; },
}:
pkgs.callPackage ./package.nix {
  inherit (import "${inputs.now}/nix" { inherit pkgs; }) now now-step;
  supervisord = pkgs.callPackage ./nix/supervisord/package.nix { };
  supervisord-now = pkgs.callPackage ./rust/package.nix {
    web = pkgs.callPackage ./web/package.nix { };
  };
  inherit (inputs) nixpkgs;
}
