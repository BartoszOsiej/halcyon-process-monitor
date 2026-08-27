import { memo, useMemo } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from "recharts";
import type { RateSample } from "../types";

export const RateChart = memo(function RateChart({
  rates,
  smoothExec,
  smoothOpen,
  smoothAlert,
}: {
  rates: RateSample[];
  smoothExec: number;
  smoothOpen: number;
  smoothAlert: number;
}) {
  const data = useMemo(() => {
    return rates.map((r, i) => ({
      idx: i,
      exec: r.exec_count,
      open: r.open_count,
      alert: r.alert_count,
    }));
  }, [rates]);

  const maxY = useMemo(() => {
    let m = 1;
    for (const d of data) {
      if (d.exec > m) m = d.exec;
      if (d.open > m) m = d.open;
      if (d.alert > m) m = d.alert;
    }
    return Math.ceil(m * 1.1);
  }, [data]);

  return (
    <div className="panel chart-panel">
      <div className="panel-header">
        EVENT RATE
        <span style={{ fontSize: 10, color: "var(--dim)", marginLeft: 12 }}>
          exec:
          <span style={{ color: "var(--blue)" }}>{smoothExec.toFixed(0)}</span>/s
          &nbsp; open:
          <span style={{ color: "var(--green)" }}>{smoothOpen.toFixed(0)}</span>/s
          &nbsp; alert:
          <span style={{ color: "var(--red)" }}>{smoothAlert.toFixed(0)}</span>/s
        </span>
      </div>
      <div className="panel-body">
        {data.length < 2 ? (
          <div className="empty">waiting for rate data…</div>
        ) : (
          <ResponsiveContainer width="100%" height={120}>
            <LineChart data={data}>
              <CartesianGrid strokeDasharray="3 3" stroke="var(--border)" />
              <XAxis
                dataKey="idx"
                tick={{ fontSize: 10, fill: "var(--dim)" }}
                stroke="var(--border)"
              />
              <YAxis
                domain={[0, maxY]}
                tick={{ fontSize: 10, fill: "var(--dim)" }}
                stroke="var(--border)"
              />
              <Tooltip
                contentStyle={{
                  background: "var(--panel)",
                  border: "1px solid var(--border)",
                  borderRadius: 4,
                  fontSize: 11,
                }}
              />
              <Legend
                wrapperStyle={{ fontSize: 10, color: "var(--dim)" }}
              />
              <Line
                type="monotone"
                dataKey="exec"
                stroke="var(--blue)"
                dot={false}
                strokeWidth={1.5}
              />
              <Line
                type="monotone"
                dataKey="open"
                stroke="var(--green)"
                dot={false}
                strokeWidth={1.5}
              />
              <Line
                type="monotone"
                dataKey="alert"
                stroke="var(--red)"
                dot={false}
                strokeWidth={1.5}
              />
            </LineChart>
          </ResponsiveContainer>
        )}
      </div>
    </div>
  );
});
