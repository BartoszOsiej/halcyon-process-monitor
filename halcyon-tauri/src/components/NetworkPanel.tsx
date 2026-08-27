import { memo, useMemo } from "react";
import type { NetworkEntry } from "../types";

interface AggregatedProc {
  comm: string;
  connect: number;
  accept: number;
  send: number;
  recv: number;
}

export const NetworkPanel = memo(function NetworkPanel({
  network,
}: {
  network: NetworkEntry[];
}) {
  const aggregated = useMemo(() => {
    const map = new Map<string, AggregatedProc>();
    for (const e of network) {
      let proc = map.get(e.comm);
      if (!proc) {
        proc = { comm: e.comm, connect: 0, accept: 0, send: 0, recv: 0 };
        map.set(e.comm, proc);
      }
      switch (e.kind) {
        case "connect": proc.connect++; break;
        case "accept": proc.accept++; break;
        case "sendto": proc.send++; break;
        case "recvfrom": proc.recv++; break;
      }
    }
    return Array.from(map.values())
      .sort(
        (a, b) =>
          b.connect + b.accept + b.send + b.recv -
          (a.connect + a.accept + a.send + a.recv)
      )
      .slice(0, 20);
  }, [network]);

  const maxTotal = Math.max(
    1,
    ...aggregated.map((p) => p.connect + p.accept + p.send + p.recv)
  );

  return (
    <div className="panel">
      <div className="panel-header">
        NETWORK <span className="count">{network.length}</span>
      </div>
      <div className="panel-body">
        {aggregated.length === 0 && (
          <div className="empty">no network events captured</div>
        )}
        {aggregated.map((p, i) => {
          const total = p.connect + p.accept + p.send + p.recv;
          const pct = Math.min(100, (total / maxTotal) * 100);
          return (
            <div key={i} className="proc-row">
              <span className="comm" style={{ minWidth: 90 }}>
                {p.comm}
              </span>
              <span
                style={{ color: "var(--blue)", fontSize: 11, width: 60, textAlign: "right" }}
              >
                &gt;{p.connect}
              </span>
              <span
                style={{ color: "var(--green)", fontSize: 11, width: 60, textAlign: "right" }}
              >
                &lt;{p.accept}
              </span>
              <div className="bar-container">
                <div
                  className="bar-fill"
                  style={{
                    width: `${pct}%`,
                    background: `linear-gradient(90deg, var(--blue), var(--purple))`,
                  }}
                />
              </div>
              <span className="opens">{total}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
});
