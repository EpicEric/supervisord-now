{
  buildGoModule,
  fetchFromGitHub,
}:
buildGoModule (finalAttrs: {
  pname = "supervisord";
  version = "0.7.4";

  src = fetchFromGitHub {
    owner = "ochinchina";
    repo = "supervisord";
    tag = "v${finalAttrs.version}";
    hash = "sha256-q9D+58xpodWJZUQr5ni6h0Gz/wTKXg94X7h4EAEMG7o=";
  };

  __structuredAttrs = true;

  proxyVendor = true;
  vendorHash = "sha256-3dynMrOMiyc4P2ZCNmCgiyJx1maWIptrMzQr8iYVWCY=";

  preBuild = "go mod tidy";
  subPackages = [ "." ];

  meta.mainProgram = "supervisord";
})
