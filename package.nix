{
  dockerTools,
  lib,
  now,
  supervisord,
  writeText,
}:
let
  supervisordConf = writeText "supervisor.conf" ''
    [unix_http_server]
    file=/supervisor.sock
    chmod=0700
  '';
in
dockerTools.buildLayeredImage {
  name = "supervisord-now";
  tag = "latest";

  contents = [
    now
    supervisord
  ];

  config = {
    Cmd = [
      "${lib.getExe supervisord}"
      "-c"
      supervisordConf
    ];
    ExposedPorts = {
      "9991" = { };
    };
  };
}
