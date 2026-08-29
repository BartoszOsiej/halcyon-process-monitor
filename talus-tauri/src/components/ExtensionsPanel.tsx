import { memo } from "react";
import type { ExtensionCount } from "../types";

function extColor(ext: string): string {
  switch (ext) {
    case "enc":
    case "locked":
    case "crypto":
      return "var(--red)";
    case "rs":
    case "py":
    case "js":
    case "ts":
    case "c":
    case "cpp":
    case "go":
    case "java":
      return "var(--green)";
    case "pdf":
    case "doc":
    case "docx":
    case "txt":
    case "md":
      return "var(--amber)";
    case "jpg":
    case "png":
    case "mp4":
    case "mp3":
      return "var(--purple)";
    case "json":
    case "toml":
    case "yaml":
    case "yml":
      return "var(--cyan)";
    default:
      return "var(--blue)";
  }
}

export const ExtensionsPanel = memo(function ExtensionsPanel({
  extensions,
}: {
  extensions: ExtensionCount[];
}) {
  const sorted = [...extensions].sort((a, b) => b.count - a.count).slice(0, 12);
  const max = Math.max(1, ...sorted.map((e) => e.count));

  return (
    <div className="panel">
      <div className="panel-header">
        FILE TYPES <span className="count">{extensions.length}</span>
      </div>
      <div className="panel-body">
        {sorted.length === 0 && (
          <div className="empty">no extension data</div>
        )}
        {sorted.map((e, i) => {
          const pct = Math.min(100, (e.count / max) * 100);
          return (
            <div key={i} className="ext-bar">
              <span className="label">.{e.extension}</span>
              <div className="bar-container">
                <div
                  className="bar-fill"
                  style={{
                    width: `${pct}%`,
                    background: extColor(e.extension),
                  }}
                />
              </div>
              <span className="count">{e.count}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
});
