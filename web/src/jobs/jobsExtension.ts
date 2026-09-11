import type * as vscode from "vscode";
import jobsIconUrl from "./jobs.svg?url";
import { api, type EvalResponse, type JobStatus } from "../api";

const VIEW_ID = "supervisordNow.jobs";
const POLL_MS = 3000;
const BACKOFF_MAX_MS = 30_000;
const VARS_STORAGE_KEY = "supervisord-now:vars";
const SECRETS_STORAGE_KEY = "supervisord-now:secrets";

type VscodeApi = typeof vscode;
type EnvironmentKind = "vars" | "secrets";

type JobNode = {
  type: "job";
  name: string;
  needs?: string[];
  status?: JobStatus;
};

type EnvironmentGroupNode = {
  type: "environmentGroup";
  kind: EnvironmentKind;
};

type EnvironmentEntryNode = {
  type: "environmentEntry";
  kind: EnvironmentKind;
  name: string;
  value: string;
};

type MessageNode = {
  type: "message";
  label: string;
  error?: boolean;
};

type JobsNode = JobNode | EnvironmentGroupNode | EnvironmentEntryNode | MessageNode;

export const jobsExtension = {
  config: {
    name: "jobs",
    publisher: "supervisord-now",
    version: "1.0.0",
    engines: {
      vscode: "*",
    },
    contributes: {
      viewsContainers: {
        activitybar: [
          {
            id: "supervisordNow",
            title: "Jobs",
            icon: "./jobs.svg",
          },
        ],
      },
      views: {
        supervisordNow: [
          {
            id: VIEW_ID,
            name: "Jobs",
          },
        ],
      },
      commands: [
        {
          command: "supervisordNow.jobs.refresh",
          title: "Refresh Jobs",
          icon: "$(refresh)",
        },
        {
          command: "supervisordNow.jobs.run",
          title: "Run Job",
          icon: "$(play)",
        },
        {
          command: "supervisordNow.jobs.stop",
          title: "Stop Job",
          icon: "$(debug-stop)",
        },
        {
          command: "supervisordNow.jobs.logs",
          title: "Show Job Logs",
          icon: "$(output)",
        },
        {
          command: "supervisordNow.jobs.addVariable",
          title: "Add Variable",
          icon: "$(add)",
        },
        {
          command: "supervisordNow.jobs.addSecret",
          title: "Add Secret",
          icon: "$(add)",
        },
        {
          command: "supervisordNow.jobs.editEnvironment",
          title: "Edit",
          icon: "$(edit)",
        },
        {
          command: "supervisordNow.jobs.removeEnvironment",
          title: "Remove",
          icon: "$(trash)",
        },
      ],
      menus: {
        "view/title": [
          {
            command: "supervisordNow.jobs.refresh",
            when: `view == ${VIEW_ID}`,
            group: "navigation",
          },
        ],
        "view/item/context": [
          {
            command: "supervisordNow.jobs.run",
            when: `view == ${VIEW_ID} && viewItem == job`,
            group: "inline@1",
          },
          {
            command: "supervisordNow.jobs.stop",
            when: `view == ${VIEW_ID} && viewItem == job`,
            group: "inline@2",
          },
          {
            command: "supervisordNow.jobs.logs",
            when: `view == ${VIEW_ID} && viewItem == job`,
            group: "inline@3",
          },
          {
            command: "supervisordNow.jobs.addVariable",
            when: `view == ${VIEW_ID} && viewItem == environmentGroup.vars`,
            group: "inline",
          },
          {
            command: "supervisordNow.jobs.addSecret",
            when: `view == ${VIEW_ID} && viewItem == environmentGroup.secrets`,
            group: "inline",
          },
          {
            command: "supervisordNow.jobs.editEnvironment",
            when: `view == ${VIEW_ID} && viewItem == environmentEntry`,
            group: "inline@1",
          },
          {
            command: "supervisordNow.jobs.removeEnvironment",
            when: `view == ${VIEW_ID} && viewItem == environmentEntry`,
            group: "inline@2",
          },
        ],
      },
    },
  },
  filesOrContents: new Map([["/jobs.svg", new URL(jobsIconUrl, window.location.href)]]),
};

export function startJobsExtension(editor: VscodeApi): vscode.Disposable {
  const provider = new JobsTreeProvider(editor);
  const tree = editor.window.createTreeView(VIEW_ID, {
    treeDataProvider: provider,
    showCollapseAll: true,
  });
  const disposables: vscode.Disposable[] = [
    provider,
    tree,
    editor.commands.registerCommand("supervisordNow.jobs.refresh", () => provider.refresh()),
    editor.commands.registerCommand("supervisordNow.jobs.run", (node: JobNode) =>
      provider.run(node),
    ),
    editor.commands.registerCommand("supervisordNow.jobs.stop", (node: JobNode) =>
      provider.stop(node),
    ),
    editor.commands.registerCommand("supervisordNow.jobs.logs", (node: JobNode) =>
      provider.showLogs(node),
    ),
    editor.commands.registerCommand("supervisordNow.jobs.addVariable", () =>
      provider.addEnvironment("vars"),
    ),
    editor.commands.registerCommand("supervisordNow.jobs.addSecret", () =>
      provider.addEnvironment("secrets"),
    ),
    editor.commands.registerCommand(
      "supervisordNow.jobs.editEnvironment",
      (node: EnvironmentEntryNode) => provider.editEnvironment(node),
    ),
    editor.commands.registerCommand(
      "supervisordNow.jobs.removeEnvironment",
      (node: EnvironmentEntryNode) => provider.removeEnvironment(node),
    ),
  ];

  void provider.refresh();

  return new editor.Disposable(() => {
    for (const disposable of disposables) {
      disposable.dispose();
    }
  });
}

class JobsTreeProvider implements vscode.TreeDataProvider<JobsNode>, vscode.Disposable {
  private readonly changeEmitter: vscode.EventEmitter<JobsNode | undefined | null | void>;
  readonly onDidChangeTreeData: vscode.Event<JobsNode | undefined | null | void>;

  private evalResult: EvalResponse | null = null;
  private statuses: JobStatus[] = [];
  private delay = POLL_MS;
  private timeout: ReturnType<typeof setTimeout> | undefined;
  private disposed = false;
  private readonly outputChannels = new Map<string, vscode.OutputChannel>();
  private readonly logSockets = new Map<string, WebSocket>();

  constructor(private readonly editor: VscodeApi) {
    this.changeEmitter = new editor.EventEmitter<JobsNode | undefined | null | void>();
    this.onDidChangeTreeData = this.changeEmitter.event;
  }

  getTreeItem(node: JobsNode): vscode.TreeItem {
    if (node.type === "job") {
      const item = new this.editor.TreeItem(node.name, this.editor.TreeItemCollapsibleState.None);
      const state = node.status?.statename ?? "Not created";
      item.description = state;
      item.tooltip = node.needs?.length
        ? `${node.status?.description ?? state}\nNeeds: ${node.needs.join(", ")}`
        : (node.status?.description ?? state);
      item.contextValue = "job";
      item.iconPath = new this.editor.ThemeIcon(statusIcon(state));
      return item;
    }

    if (node.type === "environmentGroup") {
      const values = readEnvironment(node.kind);
      const item = new this.editor.TreeItem(
        node.kind === "vars" ? "Variables" : "Secrets",
        this.editor.TreeItemCollapsibleState.Collapsed,
      );
      item.description = `${Object.keys(values).length}`;
      item.contextValue = `environmentGroup.${node.kind}`;
      item.iconPath = new this.editor.ThemeIcon(node.kind === "vars" ? "symbol-variable" : "key");
      return item;
    }

    if (node.type === "environmentEntry") {
      const item = new this.editor.TreeItem(node.name, this.editor.TreeItemCollapsibleState.None);
      item.description = node.kind === "secrets" ? "••••••" : node.value;
      item.contextValue = "environmentEntry";
      item.iconPath = new this.editor.ThemeIcon(node.kind === "secrets" ? "key" : "symbol-field");
      return item;
    }

    const item = new this.editor.TreeItem(node.label, this.editor.TreeItemCollapsibleState.None);
    item.iconPath = new this.editor.ThemeIcon(node.error ? "error" : "info");
    return item;
  }

  getChildren(node?: JobsNode): JobsNode[] {
    if (node?.type === "environmentGroup") {
      return Object.entries(readEnvironment(node.kind))
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([name, value]) => ({
          type: "environmentEntry",
          kind: node.kind,
          name,
          value,
        }));
    }

    if (node) {
      return [];
    }

    const result: JobsNode[] = [];
    if (!this.evalResult) {
      result.push({ type: "message", label: "Evaluating workflow..." });
    } else if (!this.evalResult.ok) {
      result.push({ type: "message", label: this.evalResult.error, error: true });
    } else if (this.evalResult.mode === "empty") {
      result.push({ type: "message", label: "No now.nix or flake.nix in workspace." });
    } else if (this.evalResult.jobs.length === 0) {
      result.push({ type: "message", label: "No jobs in workflow." });
    } else {
      const statusByName = new Map(this.statuses.map((status) => [status.name, status]));
      result.push(
        ...this.evalResult.jobs.map((job): JobNode => ({
          type: "job",
          name: job.name,
          needs: job.needs,
          status: statusByName.get(job.name),
        })),
      );
    }

    result.push(
      { type: "environmentGroup", kind: "vars" },
      { type: "environmentGroup", kind: "secrets" },
    );
    return result;
  }

  async refresh(): Promise<void> {
    if (this.timeout) {
      clearTimeout(this.timeout);
      this.timeout = undefined;
    }

    try {
      const [evalResult, statuses] = await Promise.all([api.evalWorkflow(), api.jobs()]);
      this.evalResult = evalResult;
      this.statuses = statuses;
      this.delay = evalResult.ok ? POLL_MS : Math.min(this.delay * 2, BACKOFF_MAX_MS);
    } catch (error) {
      this.evalResult = { ok: false, error: errorMessage(error) };
      this.delay = Math.min(this.delay * 2, BACKOFF_MAX_MS);
    }

    this.changeEmitter.fire();
    if (!this.disposed) {
      this.timeout = setTimeout(() => void this.refresh(), this.delay);
    }
  }

  async run(node: JobNode): Promise<void> {
    if (node?.type !== "job") return;
    try {
      await api.invoke(node.name, readEnvironment("vars"), readEnvironment("secrets"));
      await this.refresh();
    } catch (error) {
      void this.editor.window.showErrorMessage(
        `Could not run ${node.name}: ${errorMessage(error)}`,
      );
    }
  }

  async stop(node: JobNode): Promise<void> {
    if (node?.type !== "job") return;
    try {
      await api.stop(node.name);
      await this.refresh();
    } catch (error) {
      void this.editor.window.showErrorMessage(
        `Could not stop ${node.name}: ${errorMessage(error)}`,
      );
    }
  }

  async showLogs(node: JobNode): Promise<void> {
    if (node?.type !== "job") return;
    const channel =
      this.outputChannels.get(node.name) ??
      this.editor.window.createOutputChannel(`Job: ${node.name}`);
    this.outputChannels.set(node.name, channel);
    const panel = document.querySelector<HTMLElement>("#panel");
    if (panel && getComputedStyle(panel).display === "none") {
      await this.editor.commands.executeCommand("workbench.action.togglePanel");
    }
    channel.show(true);

    this.logSockets.get(node.name)?.close();
    const httpUrl = new URL(api.logsUrl(node.name), window.location.href);
    httpUrl.protocol = httpUrl.protocol === "https:" ? "wss:" : "ws:";
    const socket = new WebSocket(httpUrl);
    this.logSockets.set(node.name, socket);
    socket.onmessage = (event) => {
      const payload = JSON.parse(String(event.data)) as { data?: string; error?: string };
      if (payload.data) channel.append(payload.data);
      if (payload.error) channel.appendLine(`[${payload.error}]`);
    };
    socket.onerror = () => channel.appendLine("[log connection failed]");
    socket.onclose = () => {
      if (this.logSockets.get(node.name) === socket) {
        this.logSockets.delete(node.name);
      }
    };
  }

  async addEnvironment(kind: EnvironmentKind): Promise<void> {
    const name = await this.editor.window.showInputBox({
      title: kind === "vars" ? "Add variable" : "Add secret",
      prompt: "Environment variable name",
      validateInput: validateEnvironmentName,
    });
    if (!name) return;

    const values = readEnvironment(kind);
    const value = await this.editor.window.showInputBox({
      title: kind === "vars" ? `Value for ${name}` : `Secret value for ${name}`,
      password: kind === "secrets",
      value: kind === "vars" ? (values[name] ?? "") : "",
    });
    if (value === undefined) return;

    values[name] = value;
    writeEnvironment(kind, values);
    this.changeEmitter.fire();
  }

  async editEnvironment(node: EnvironmentEntryNode): Promise<void> {
    if (node?.type !== "environmentEntry") return;
    const value = await this.editor.window.showInputBox({
      title: `Value for ${node.name}`,
      password: node.kind === "secrets",
      value: node.kind === "vars" ? node.value : "",
    });
    if (value === undefined) return;

    const values = readEnvironment(node.kind);
    values[node.name] = value;
    writeEnvironment(node.kind, values);
    this.changeEmitter.fire();
  }

  async removeEnvironment(node: EnvironmentEntryNode): Promise<void> {
    if (node?.type !== "environmentEntry") return;
    const action = await this.editor.window.showWarningMessage(
      `Remove ${node.name}?`,
      { modal: true },
      "Remove",
    );
    if (action !== "Remove") return;

    const values = readEnvironment(node.kind);
    delete values[node.name];
    writeEnvironment(node.kind, values);
    this.changeEmitter.fire();
  }

  dispose(): void {
    this.disposed = true;
    if (this.timeout) clearTimeout(this.timeout);
    for (const socket of this.logSockets.values()) socket.close();
    for (const channel of this.outputChannels.values()) channel.dispose();
    this.logSockets.clear();
    this.outputChannels.clear();
    this.changeEmitter.dispose();
  }
}

function statusIcon(state: string): string {
  switch (state.toUpperCase()) {
    case "RUNNING":
      return "play-circle";
    case "STARTING":
    case "STOPPING":
    case "BACKOFF":
      return "loading~spin";
    case "EXITED":
      return "pass";
    case "FATAL":
      return "error";
    default:
      return "circle-outline";
  }
}

function storageKey(kind: EnvironmentKind): string {
  return kind === "vars" ? VARS_STORAGE_KEY : SECRETS_STORAGE_KEY;
}

function readEnvironment(kind: EnvironmentKind): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of (localStorage.getItem(storageKey(kind)) ?? "").split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const equals = trimmed.indexOf("=");
    if (equals <= 0) continue;
    result[trimmed.slice(0, equals).trim()] = trimmed.slice(equals + 1).trim();
  }
  return result;
}

function writeEnvironment(kind: EnvironmentKind, values: Record<string, string>): void {
  const content = Object.entries(values)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([name, value]) => `${name}=${value}`)
    .join("\n");
  localStorage.setItem(storageKey(kind), content);
}

function validateEnvironmentName(value: string): string | undefined {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(value)) {
    return "Use letters, numbers, and underscores; the first character cannot be a number.";
  }
  return undefined;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
