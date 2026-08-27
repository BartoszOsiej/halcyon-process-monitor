import { memo } from "react";
import type { ProcessInfo } from "../types";

export const ProcessesPanel = memo(function ProcessesPanel({
  processes,
}: {
  processes: ProcessInfo[];
}) {
  const maxOpens = Math.max(1, ...processes.map((p) => p.total_opens || p.window_opens || 0));

  return (
    <div className="panel">
      <div className="panel-header">
        PROCESSES <span className="count">{processes.length}</span>
      </div>
      <div className="panel-body">
        {processes.length === 0 && (
          <div className="empty">no process data yet</div>
        )}
        {processes.map((p, i) => {
          const opens = p.window_opens || p.total_opens;
          const pct = Math.min(100, (opens / maxOpens) * 100);
          const barColor =
            p.alerts > 0
              ? "var(--red)"
              : opens > maxOpens * 0.7
                ? "var(--amber)"
                : "var(--blue)";

          return (
            <div key={i} className="proc-row">
              <span className="pid">{p.pid}</span>
              <span className="comm">{p.comm}</span>
              <div className="bar-container">
                <div
                  className="bar-fill"
                  style={{
                    width: `${pct}%`,
                    background: barColor,
                  }}
                />
              </div>
              <span className="opens">{opens}</span>
              <span className="alerts-count">
                {p.alerts > 0 ? `⚠${p.alerts}` : ""}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
});
