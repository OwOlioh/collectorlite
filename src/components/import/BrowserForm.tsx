import { Upload } from "lucide-react";
import type { VideoItem } from "../../types";

interface BrowserFormProps {
  browserFileName: string;
  browserItems: VideoItem[];
  onFileDrop: (e: React.DragEvent) => void;
  onFileInput: (e: React.ChangeEvent<HTMLInputElement>) => void;
}

export function BrowserForm({ browserFileName, browserItems, onFileDrop, onFileInput }: BrowserFormProps) {
  return (
    <div className="browser-block">
      <label className="field-label">浏览器书签文件</label>
      <div className="browser-drop-zone"
        onDragOver={(e) => e.preventDefault()}
        onDrop={onFileDrop}>
        <Upload size={24} />
        <p>拖拽书签 HTML 文件到此处，或点击选择文件。</p>
        <input type="file" accept=".html,.htm" onChange={onFileInput} className="browser-file-input" />
      </div>
      {browserFileName && (
        <div className="parsed-card">
          <strong>{browserFileName}</strong>
          <span>{browserItems.length} 条</span>
        </div>
      )}
    </div>
  );
}