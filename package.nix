{
  cacert,
  dockerTools,
  lib,
  nil,
  nix,
  nixfmt-rs,
  nixpkgs,
  now,
  runCommand,
  supervisord,
  supervisord-now,
  writeText,
  writeTextDir,
}:
let
  nixConf = writeTextDir "/etc/nix/nix.conf" ''
    build-users-group =
    sandbox = false
  '';

  jobsConfDir = runCommand "supervisord-jobs-conf" { } ''
    mkdir -p $out/etc/supervisord-now $out/var/lib/supervisord-now $out/workspace
    touch $out/etc/supervisord-now/.keep
    touch $out/var/lib/supervisord-now/.keep
  '';

  supervisordConf = writeText "supervisor.conf" ''
    [unix_http_server]
    file=/supervisor.sock
    chmod=0700

    [program:supervisord-now]
    command=supervisord-now
    autostart=true
    autorestart=true
    startsecs=0
    stdout_logfile=/dev/stdout
    stdout_logfile_maxbytes=0
    redirect_stderr=true

    [include]
    files=/etc/supervisord-now/*.conf
  '';
in
dockerTools.buildLayeredImage {
  name = "supervisord-now";
  tag = "latest";

  contents = [
    supervisord-now
    supervisord
    now
    nix
    nil
    nixfmt-rs
    nixConf
    jobsConfDir
  ];

  config = {
    Cmd = [
      "${lib.getExe supervisord}"
      "-c"
      supervisordConf
    ];
    Env = [
      "NIX_PATH=nixpkgs=${nixpkgs}"
      "NIX_SSL_CERT_FILE=${cacert}/etc/ssl/certs/ca-bundle.crt"
    ];
    ExposedPorts = {
      "9991" = { };
    };
    Volumes = {
      "/workspace" = { };
    };
  };
}
