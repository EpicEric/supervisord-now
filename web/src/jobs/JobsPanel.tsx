import { useCallback, useEffect, useRef, useState } from "react";
import { api, type EvalResponse, type JobStatus } from "../api";

const STATE_CLASS: Record<string, string> = {
  RUNNING: "running",
  STOPPED: "stopped",
  EXITED: "exited",
  FATAL: "fatal",
  STARTING: "starting",
  BACKOFF: "backoff",
  STOPPING: "stopping",
};

export function JobsPanel() {
  const [evalResult, setEvalResult] = useState<EvalResponse | null>(null);
  const [statuses, setStatuses] = useState<JobStatus[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [evalResponse, jobData] = await Promise.all([api.evalWorkflow(), api.jobs()]);
      setEvalResult(evalResponse);
      setStatuses(jobData);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = setInterval(() => void refresh(), 3000);
    return () => clearInterval(interval);
  }, [refresh]);

  const invoke = async (name: string) => {
    setBusy(name);
    setError(null);
    try {
      const { vars, secrets } = parseEnvTexts();
      await api.invoke(name, vars, secrets);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const stop = async (name: string) => {
    setBusy(name);
    setError(null);
    try {
      await api.stop(name);
      await refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(null);
    }
  };

  const statusByName = new Map(statuses.map((s) => [s.name, s]));

  return (
    <aside id="jobs-panel">
      <header>
        <h2>Jobs</h2>
        {evalResult?.ok && <span className="mode">{evalResult.mode}</span>}
      </header>

      {error && <div className="error">{error}</div>}

      <ul>
        {(evalResult?.ok ? evalResult.jobs : []).map((job) => {
          const status = statusByName.get(job.name);
          const state = status?.statename ?? "NOT CREATED";
          return (
            <li key={job.name}>
              <div className="job-row">
                <span
                  className={`state ${STATE_CLASS[state] ?? "unknown"}`}
                  title={status?.description}
                >
                  {state}
                </span>
                <span
                  className="name"
                  title={job.needs?.length ? `needs: ${job.needs.join(", ")}` : ""}
                >
                  {job.name}
                </span>
                <button
                  disabled={busy === job.name || !evalResult?.ok}
                  onClick={() => void invoke(job.name)}
                >
                  {busy === job.name ? "..." : "Run"}
                </button>
                <button disabled={busy === job.name} onClick={() => void stop(job.name)}>
                  Stop
                </button>
                <button
                  className="log-toggle"
                  onClick={() => setExpanded(expanded === job.name ? null : job.name)}
                >
                  {expanded === job.name ? "Hide" : "Logs"}
                </button>
              </div>
              {expanded === job.name && <LogViewer job={job.name} />}
            </li>
          );
        })}
      </ul>

      {evalResult && !evalResult.ok && <pre className="error">{evalResult.error}</pre>}
      {evalResult?.ok && evalResult.jobs.length === 0 && <p>No jobs in workflow.</p>}
      {!evalResult && <p>Evaluating workflow...</p>}

      <EnvEditor />
    </aside>
  );
}

const VARS_STORAGE_KEY = "supervisord-now:vars";
const SECRETS_STORAGE_KEY = "supervisord-now:secrets";

function parseEnvTexts(): { vars: Record<string, string>; secrets: Record<string, string> } {
  return {
    vars: parseEnvText(localStorage.getItem(VARS_STORAGE_KEY) ?? ""),
    secrets: parseEnvText(localStorage.getItem(SECRETS_STORAGE_KEY) ?? ""),
  };
}

function parseEnvText(text: string): Record<string, string> {
  const result: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith("#")) continue;
    const eq = trimmed.indexOf("=");
    if (eq <= 0) continue;
    result[trimmed.slice(0, eq).trim()] = trimmed.slice(eq + 1).trim();
  }
  return result;
}

function EnvEditor() {
  const [open, setOpen] = useState(false);
  const [vars, setVars] = useState(() => localStorage.getItem(VARS_STORAGE_KEY) ?? "");
  const [secrets, setSecrets] = useState(() => localStorage.getItem(SECRETS_STORAGE_KEY) ?? "");

  useEffect(() => {
    localStorage.setItem(VARS_STORAGE_KEY, vars);
  }, [vars]);
  useEffect(() => {
    localStorage.setItem(SECRETS_STORAGE_KEY, secrets);
  }, [secrets]);

  return (
    <section className="env-editor">
      <button onClick={() => setOpen(!open)}>
        {open ? "Hide variables" : "Variables & secrets"}
      </button>
      {open && (
        <>
          <label>
            Variables (KEY=value per line)
            <textarea rows={4} value={vars} onChange={(e) => setVars(e.target.value)} />
          </label>
          <label>
            Secrets (KEY=value per line)
            <textarea rows={4} value={secrets} onChange={(e) => setSecrets(e.target.value)} />
          </label>
        </>
      )}
    </section>
  );
}

function LogViewer({ job }: { job: string }) {
  const ref = useRef<HTMLPreElement>(null);
  const [lines, setLines] = useState<string>("");

  useEffect(() => {
    const socket = new WebSocket(api.logsUrl(job));
    let buffer = "";
    socket.onmessage = (event) => {
      const payload = JSON.parse(event.data);
      if (payload.data) {
        buffer += payload.data;
        if (buffer.length > 256 * 1024) {
          buffer = buffer.slice(-128 * 1024);
        }
        setLines(buffer);
      } else if (payload.error) {
        setLines(buffer + `\n[${payload.error}]\n`);
      }
    };
    return () => socket.close();
  }, [job]);

  useEffect(() => {
    if (ref.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [lines]);

  return (
    <pre className="log-viewer" ref={ref}>
      {lines || "(no output)"}
    </pre>
  );
}
