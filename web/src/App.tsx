import { useEffect, useRef } from "react";
import { startWorkbench } from "./workbench";
import { JobsPanel } from "./jobs/JobsPanel";

export function App() {
  const containerRef = useRef<HTMLDivElement>(null);
  const startedRef = useRef(false);

  useEffect(() => {
    if (startedRef.current || !containerRef.current) return;
    startedRef.current = true;
    void startWorkbench(containerRef.current);
  }, []);

  return (
    <div id="app">
      <div id="workbench-container" ref={containerRef} />
      <JobsPanel />
    </div>
  );
}
