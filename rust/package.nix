{
  frontend,
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage (finalAttrs: {
  version = (lib.importTOML ./Cargo.toml).package.version;

  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./src
      ./Cargo.toml
      ./Cargo.lock
    ];
  };

  cargoLock.lockFile = ./Cargo.lock;

  strictDeps = true;
  __structuredAttrs = true;

  preBuild = ''
    rm -rf src/frontend
    ln -s ${frontend} src/frontend
  '';

  doCheck = false;

  meta.mainProgram = "supervisord-now";
})
