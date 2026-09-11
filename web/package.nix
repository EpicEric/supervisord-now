{
  buildNpmPackage,
  importNpmLock,
  lib,
}:
buildNpmPackage {
  pname = "supervisord-now-web";
  version = (lib.importJSON ./package.json).version;

  src = ./.;

  npmDeps = importNpmLock {
    npmRoot = ./.;
  };

  strictDeps = true;
  __structuredAttrs = true;

  inherit (importNpmLock) npmConfigHook;

  installPhase = ''
    runHook preInstall
    mkdir -p $out
    cp -r dist/ $out/dist/
    runHook postInstall
  '';
}
