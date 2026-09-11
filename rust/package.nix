{
  rustPlatform,
  lib,
  web,
}:
rustPlatform.buildRustPackage {
  pname = "supervisord-now";
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

  postPatch = ''
    substituteInPlace src/main.rs \
      --replace-fail '$CARGO_MANIFEST_DIR/../web/dist' '${web}/dist'
  '';

  meta.mainProgram = "supervisord-now";
}
