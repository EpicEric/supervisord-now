const WORKSPACE_ROOT = "/workspace";

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, init);
  if (!response.ok) {
    let message = `${response.status}`;
    try {
      const body = await response.json();
      if (body.error) message = body.error;
    } catch {
      message = `${response.status} ${response.statusText}`;
    }
    throw new Error(message);
  }
  if (response.status === 204) {
    return undefined as T;
  }
  return response.json() as Promise<T>;
}

const encode = (path: string) => encodeURIComponent(path);

export type EvalResponse =
  | { ok: true; mode: string; jobs: { id: string; name: string; needs?: string[] }[] }
  | { ok: false; error: string };

export type JobStatus = {
  id: string;
  group: string;
  state: number;
  statename: string;
  description: string;
  exitstatus: number;
  spawnerr: string;
};

export const api = {
  tree: () => fetch(`/api/files`).then((r) => r.json()),
  readFile: (path: string) => request<{ content: string }>(`/api/file?path=${encode(path)}`),
  writeFile: (path: string, content: string) =>
    request<void>(`/api/file?path=${encode(path)}`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ content }),
    }),
  deleteFile: (path: string) =>
    request<void>(`/api/file?path=${encode(path)}`, { method: "DELETE" }),
  mkdir: (path: string) => request<void>(`/api/dir?path=${encode(path)}`, { method: "POST" }),
  rename: (from: string, to: string) =>
    request<void>(`/api/rename`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ from, to }),
    }),
  evalWorkflow: async (): Promise<EvalResponse> => {
    const response = await fetch(`/api/eval`);
    const body = await response.json();
    if (!response.ok) {
      return { ok: false, error: body.error ?? `HTTP ${response.status}` };
    }
    return { ok: true, mode: body.mode, jobs: body.jobs };
  },
  jobs: (): Promise<JobStatus[]> => fetch(`/api/jobs`).then((r) => r.json()),
  invoke: (name: string, vars: Record<string, string>, secrets: Record<string, string>) =>
    request<void>(`/api/jobs/${encode(name)}/invoke`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ vars, secrets }),
    }),
  stop: (name: string) => request<void>(`/api/jobs/${encode(name)}/stop`, { method: "POST" }),
  logsUrl: (name: string) => `/api/jobs/${encode(name)}/logs`,
};

export const workspaceUriToPath = (uri: string): string => {
  const prefix = `file://${WORKSPACE_ROOT}`;
  if (!uri.startsWith(prefix)) {
    throw new Error(`unexpected uri outside workspace: ${uri}`);
  }
  return uri.slice(prefix.length).replace(/^\//, "");
};
