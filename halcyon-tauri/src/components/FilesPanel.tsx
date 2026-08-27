import { memo } from "react";
import type { FileRank } from "../types";

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

export const FilesPanel = memo(function FilesPanel({
  files,
}: {
  files: FileRank[];
}) {
  const max = Math.max(1, ...files.map((f) => f.count));

  return (
    <div className="panel">
      <div className="panel-header">
        TOP FILES <span className="count">{files.length}</span>
      </div>
      <div className="panel-body">
        {files.length === 0 && (
          <div className="empty">no file data yet</div>
        )}
        {files.map((f, i) => {
          const pct = Math.min(100, (f.count / max) * 100);
          const name = f.path.split("/").pop() || f.path;
          const entropyColor =
            f.entropy > 0.7 ? "var(--red)" : f.entropy > 0.4 ? "var(--amber)" : "var(--green)";

          return (
            <div key={i} className="file-row">
              <span className="rank">{i + 1}</span>
              <span className="name">{name}</span>
              <span className="ext" style={{ color: extColor(f.extension) }}>
                .{f.extension}
              </span>
              <div className="bar-container" style={{ flex: 1 }}>
                <div
                  className="bar-fill"
                  style={{ width: `${pct}%`, background: "var(--blue)" }}
                />
              </div>
              <span className="count">{f.count}</span>
              <span className="entropy" style={{ color: entropyColor }}>
                H:{f.entropy.toFixed(1)}
              </span>
            </div>
          );
        })}
      </div>
    </div>
  );
});
