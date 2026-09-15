{
  buildGo127Module,
  fetchFromGitHub,
  versionCheckHook,
}:
buildGo127Module (finalAttrs: {
  pname = "supervisord";
  version = "0.7.5";

  src = fetchFromGitHub {
    owner = "ochinchina";
    repo = "supervisord";
    tag = "v${finalAttrs.version}";
    hash = "sha256-2mNAvQJ36aeyMBi37a/hBQOxOWDWA1/LM6dwoi0g1so=";
  };

  __structuredAttrs = true;

  proxyVendor = true;
  vendorHash = "sha256-r1almxcbVR8J49+wtgiT/DE5HUkwUl3PFusAMnjsG3c=";

  preBuild = "go mod tidy";
  subPackages = [ "." ];

  nativeInstallCheckInputs = [ versionCheckHook ];
  doInstallCheck = true;
  versionCheckProgramArg = "version";

  meta.mainProgram = "supervisord";
})
