import { memo } from "react";
import type { TalusEvent } from "../types";

function kindTag(kind?: string): { tag: string; cls: string } {
  switch (kind) {
    case "exec": return { tag: "EXEC  ", cls: "tag-exec" };
    case "open": return { tag: "OPEN  ", cls: "tag-open" };
    case "connect":
    case "accept":
    case "sendto":
    case "recvfrom": return { tag: "NET   ", cls: "tag-net" };
    case "unlink": return { tag: "UNLINK", cls: "tag-unlink" };
    case "mkdir": return { tag: "MKDIR ", cls: "tag-mkdir" };
    default: return { tag: "EVENT ", cls: "tag-exec" };
  }
}

export const EventsPanel = memo(function EventsPanel({
  events,
}: {
  events: TalusEvent[];
}) {
  return (
    <div className="panel">
      <div className="panel-header">
        EVENTS <span className="count">{events.length}</span>
      </div>
      <div className="panel-body">
        {events.length === 0 && (
          <div className="empty">waiting for events…</div>
        )}
        {events.map((e, i) => {
          if (e.type === "alert") {
            return (
              <div key={i} className="event-row">
                <span className="ts">{e.ts}</span>
                <span className="tag tag-alert">⚠ ALERT</span>
                <span className="pid">[{e.pid}]</span>
                <span className="comm">{e.comm}</span>
                <span className="file">{e.opens} opens/s</span>
              </div>
            );
          }
          if (e.type === "response") {
            return (
              <div key={i} className="event-row">
                <span className="ts">{e.ts}</span>
                <span className="tag tag-response">⚡ RESP </span>
                <span className="pid">[{e.pid}]</span>
                <span className="comm">{e.comm}</span>
                <span className="file">{e.action}</span>
              </div>
            );
          }
          const { tag, cls } = kindTag(e.kind);
          return (
            <div key={i} className="event-row">
              <span className="ts">{e.ts}</span>
              <span className={`tag ${cls}`}>{tag}</span>
              <span className="pid">[{e.pid}]</span>
              <span className="comm">{e.comm}</span>
              <span className="file">{e.file ? `→ ${e.file}` : ""}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
});
