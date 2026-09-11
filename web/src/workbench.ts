import { LogLevel } from "@codingame/monaco-vscode-api";
import getEnvironmentServiceOverride from "@codingame/monaco-vscode-environment-service-override";
import getExplorerServiceOverride from "@codingame/monaco-vscode-explorer-service-override";
import getKeybindingsServiceOverride from "@codingame/monaco-vscode-keybindings-service-override";
import getLifecycleServiceOverride from "@codingame/monaco-vscode-lifecycle-service-override";
import getOutputServiceOverride from "@codingame/monaco-vscode-output-service-override";
import outputLinkDetectionWorkerUrl from "@codingame/monaco-vscode-output-service-override/worker?worker&url";
import getPreferencesServiceOverride from "@codingame/monaco-vscode-preferences-service-override";
import getRemoteAgentServiceOverride from "@codingame/monaco-vscode-remote-agent-service-override";
import getSearchServiceOverride from "@codingame/monaco-vscode-search-service-override";
import getSecretStorageServiceOverride from "@codingame/monaco-vscode-secret-storage-service-override";
import getStorageServiceOverride from "@codingame/monaco-vscode-storage-service-override";
import getBannerServiceOverride from "@codingame/monaco-vscode-view-banner-service-override";
import getStatusBarServiceOverride from "@codingame/monaco-vscode-view-status-bar-service-override";
import getTitleBarServiceOverride from "@codingame/monaco-vscode-view-title-bar-service-override";
import { registerFileSystemOverlay } from "@codingame/monaco-vscode-files-service-override";
import { LanguageClientWrapper, type LanguageClientConfig } from "monaco-languageclient/lcwrapper";
import {
  defaultHtmlAugmentationInstructions,
  defaultViewsInit,
  MonacoVscodeApiWrapper,
  type MonacoVscodeApiConfig,
} from "monaco-languageclient/vscodeApiWrapper";
import {
  defineDefaultWorkerLoaders,
  useWorkerFactory,
  Worker as MonacoWorker,
} from "monaco-languageclient/workerFactory";
import { toSocket, WebSocketMessageReader, WebSocketMessageWriter } from "vscode-ws-jsonrpc";
import * as vscode from "vscode";
import { RestFileSystemProvider } from "./fsProvider";
import { jobsExtension, startJobsExtension } from "./jobs/jobsExtension";
import { nixExtension, nixGrammarUrl, nixLanguageConfigurationUrl } from "./nixLanguage";

const WORKSPACE_URI = vscode.Uri.file("/workspace");
const RECONNECT_DELAY_MS = 1_000;
const RECONNECT_DELAY_MAX_MS = 30_000;

export async function startWorkbench(container: HTMLElement): Promise<MonacoVscodeApiWrapper> {
  const provider = new RestFileSystemProvider();
  registerFileSystemOverlay(1, provider);

  const vscodeApiConfig: MonacoVscodeApiConfig = {
    $type: "extended",
    logLevel: LogLevel.Warning,
    serviceOverrides: {
      ...getKeybindingsServiceOverride(),
      ...getLifecycleServiceOverride(),
      ...getBannerServiceOverride(),
      ...getStatusBarServiceOverride(),
      ...getTitleBarServiceOverride(),
      ...getExplorerServiceOverride(),
      ...getRemoteAgentServiceOverride(),
      ...getEnvironmentServiceOverride(),
      ...getSecretStorageServiceOverride(),
      ...getStorageServiceOverride(),
      ...getSearchServiceOverride(),
      ...getOutputServiceOverride(),
      ...getPreferencesServiceOverride(),
    },
    viewsConfig: {
      $type: "ViewsService",
      htmlContainer: container,
      htmlAugmentationInstructions: defaultHtmlAugmentationInstructions,
      viewsInitFunc: defaultViewsInit,
    },
    workspaceConfig: {
      enableWorkspaceTrust: false,
      windowIndicator: {
        label: "supervisord-now",
        tooltip: "supervisord-now workspace",
        command: "",
      },
      workspaceProvider: {
        trusted: true,
        async open() {
          window.open(window.location.href);
          return true;
        },
        workspace: {
          folderUri: WORKSPACE_URI,
        },
      },
      configurationDefaults: {
        "window.title": "supervisord-now${separator}${dirty}${activeEditorShort}",
      },
      productConfiguration: {
        nameShort: "supervisord-now",
        nameLong: "supervisord-now",
      },
    },
    userConfiguration: {
      json: JSON.stringify({
        "workbench.colorTheme": "Default Dark Modern",
        "editor.wordBasedSuggestions": "off",
        "explorer.autoReveal": true,
        "files.autoSave": "off",
        "window.commandCenter": false,
      }),
    },
    extensions: [{ config: nixExtension }, jobsExtension],
    monacoWorkerFactory: (logger) =>
      useWorkerFactory({
        logger,
        workerLoaders: {
          ...defineDefaultWorkerLoaders(),
          OutputLinkDetectionWorker: () =>
            new MonacoWorker(outputLinkDetectionWorkerUrl, { type: "module" }),
        },
      }),
  };

  const apiWrapper = new MonacoVscodeApiWrapper(vscodeApiConfig);
  await apiWrapper.start();

  const nixRegistration = apiWrapper.getExtensionRegisterResult(nixExtension.name) as
    | { registerFileUrl?: (path: string, url: string) => unknown }
    | undefined;
  nixRegistration?.registerFileUrl?.("/nix.tmLanguage.json", nixGrammarUrl);
  nixRegistration?.registerFileUrl?.(
    "/nix-language-configuration.json",
    nixLanguageConfigurationUrl,
  );

  const jobsRegistration = apiWrapper.getExtensionRegisterResult(jobsExtension.config.name) as
    | { getApi?: () => Promise<typeof vscode> }
    | undefined;
  if (!jobsRegistration?.getApi) {
    throw new Error("jobs extension API is unavailable");
  }
  startJobsExtension(await jobsRegistration.getApi());

  await startLanguageClient();

  await vscode.commands.executeCommand("workbench.view.explorer");
  const nowNix = vscode.Uri.file("/workspace/now.nix");
  if (await fileExists(nowNix)) {
    await vscode.window.showTextDocument(nowNix);
  } else {
    const flake = vscode.Uri.file("/workspace/flake.nix");
    if (await fileExists(flake)) {
      await vscode.window.showTextDocument(flake);
    }
  }

  return apiWrapper;
}

async function startLanguageClient(): Promise<void> {
  const protocol = window.location.protocol === "https:" ? "wss" : "ws";
  const url = `${protocol}://${window.location.host}/api/lsp`;

  const banner = document.createElement("div");
  banner.id = "connection-lost-banner";
  banner.textContent = "Connection lost. Reconnecting...";
  banner.hidden = true;
  document.body.appendChild(banner);

  let delay = RECONNECT_DELAY_MS;
  let reconnectTimer: ReturnType<typeof setTimeout> | undefined;

  const scheduleReconnect = () => {
    if (reconnectTimer) return;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = undefined;
      void connect();
    }, delay);
    delay = Math.min(delay * 2, RECONNECT_DELAY_MAX_MS);
  };

  const connect = async (): Promise<void> => {
    const webSocket = new WebSocket(url);
    webSocket.addEventListener("open", () => {
      banner.hidden = true;
    });
    webSocket.addEventListener("error", () => {
      banner.hidden = false;
    });
    webSocket.addEventListener("close", scheduleReconnect);

    const iWebSocket = toSocket(webSocket);
    const reader = new WebSocketMessageReader(iWebSocket);
    const writer = new WebSocketMessageWriter(iWebSocket);

    const languageClientConfig: LanguageClientConfig = {
      languageId: "nix",
      connection: {
        options: {
          $type: "WebSocketDirect",
          webSocket,
        },
        messageTransports: { reader, writer },
      },
      clientOptions: {
        documentSelector: [{ language: "nix", scheme: "file" }],
        workspaceFolder: {
          index: 0,
          name: "workspace",
          uri: WORKSPACE_URI,
        },
      },
    };

    const lcWrapper = new LanguageClientWrapper(languageClientConfig);
    try {
      await lcWrapper.start();
      delay = RECONNECT_DELAY_MS;
    } catch {
      banner.hidden = false;
    }
  };

  await connect();
}

async function fileExists(uri: vscode.Uri): Promise<boolean> {
  try {
    await vscode.workspace.fs.stat(uri);
    return true;
  } catch {
    return false;
  }
}
