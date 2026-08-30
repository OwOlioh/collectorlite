import { Check } from "lucide-react";
import type { ImportResult } from "../../types";

interface ResultCardProps {
  result: ImportResult;
  onContinue: () => void;
}

export function ResultCard({ result, onContinue }: ResultCardProps) {
  return (
    <div className="result-card">
      <div className="result-icon">
        <Check size={28} />
      </div>
      <h2>导入完成</h2>
      <div className="result-stats">
        <span><strong>{result.total}</strong> 总计</span>
        <span><strong>{result.imported}</strong> 新增</span>
        <span><strong>{result.skipped}</strong> 跳过</span>
        <span><strong>{result.failed}</strong> 失败</span>
      </div>
      {result.errors && result.errors.length > 0 && (
        <div className="result-errors">
          <strong>部分导入失败</strong>
          <ul>
            {result.errors.map((message, index) => (
              <li key={`${index}-${message.slice(0, 30)}`}>{message}</li>
            ))}
          </ul>
        </div>
      )}
      <button className="primary-button" type="button" onClick={onContinue}>
        继续导入
      </button>
    </div>
  );
}