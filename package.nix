{
  cacert,
  dockerTools,
  lib,
  nil,
  nix,
  nixfmt-rs,
  nixpkgs,
  now,
  now-step,
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
    mkdir -p $out/var/lib/supervisord-now/conf $out/workspace
    touch $out/var/lib/supervisord-now/.keep
    touch $out/var/lib/supervisord-now/conf/.keep
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
    files=/var/lib/supervisord-now/conf/*.conf
  '';
in
dockerTools.buildLayeredImage {
  name = "supervisord-now";
  tag = "latest";

  contents = [
    supervisord-now
    supervisord
    now
    now-step
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
      "SUPERVISOR_CONF_DIR=/var/lib/supervisord-now/conf"
    ];
    ExposedPorts = {
      "9991" = { };
    };
    Volumes = {
      "/workspace" = { };
      "/var/lib/supervisord-now" = { };
    };
  };
}
