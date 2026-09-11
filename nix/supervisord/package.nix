{
  buildGoModule,
  fetchFromGitHub,
}:
buildGoModule (finalAttrs: {
  pname = "supervisord";
  version = "0.7.4-unstable-2026-08-20";

  src = fetchFromGitHub {
    owner = "ochinchina";
    repo = "supervisord";
    # RPC reply structs must use Go `int`, not `int64`: the pinned
    # gorilla-xmlrpc encoder has no int64 case and emits an empty <value>.
    # ce0722f ("logtail doesn't work #211") changed ProcessTailLog.Offset to int.
    # Do not advance past c2e0d3a ("store logs by time #324"): it added a
    # `backups <= 0` guard that makes FileLogger refuse to write or read
    # logs when backups=0 (rust/src/jobs.rs sets stdout_logfile_backups=0),
    # failing at runtime with NO_FILE.
    rev = "ce0722f7f6aac0886c5c996f8dbc581e28b9329e";
    hash = "sha256-80Nf9UyaUAjiS90v+pfdNLt02yXQcPnKKjskcDucRqk=";
  };

  __structuredAttrs = true;

  proxyVendor = true;
  vendorHash = "sha256-3dynMrOMiyc4P2ZCNmCgiyJx1maWIptrMzQr8iYVWCY=";

  preBuild = "go mod tidy";
  subPackages = [ "." ];

  meta.mainProgram = "supervisord";
})
