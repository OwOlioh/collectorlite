import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { CollectionStats, MonthCount, SourceCount, TagCountStat } from "../types";

const CHART_PALETTE = [
  "#6366f1",
  "#10b981",
  "#f59e0b",
  "#ef4444",
  "#0ea5e9",
  "#a855f7",
  "#ec4899",
  "#14b8a6",
  "#f97316",
  "#8b5cf6"
];

/** 时间范围选项：0 = 全部；30 / 90 = 近 N 天。 */
const RANGE_OPTIONS: Array<{ label: string; days: number }> = [
  { label: "全部", days: 0 },
  { label: "近 30 天", days: 30 },
  { label: "近 90 天", days: 90 }
];

/** 来源分布环形图（手写 SVG，无图表库）。扇区与图例均可点击下钻。 */
function SourceDonut({
  data,
  onDrillSource
}: {
  data: SourceCount[];
  onDrillSource?: (source: string) => void;
}) {
  const total = data.reduce((sum, d) => sum + d.count, 0);
  const radius = 52;
  const cx = 70;
  const cy = 70;
  const stroke = 22;
  const circumference = 2 * Math.PI * radius;
  let offset = 0;

  const segments = total > 0
    ? data.map((d, i) => {
        const dash = (d.count / total) * circumference;
        const segment = (
          <circle
            key={d.source}
            r={radius}
            cx={cx}
            cy={cy}
            fill="none"
            stroke={CHART_PALETTE[i % CHART_PALETTE.length]}
            strokeWidth={stroke}
            strokeDasharray={`${dash} ${circumference - dash}`}
            strokeDashoffset={-offset}
            transform={`rotate(-90 ${cx} ${cy})`}
            className="donut-seg"
            onClick={() => onDrillSource?.(d.source)}
          >
            <title>点击按「{d.source}」筛选收藏库</title>
          </circle>
        );
        offset += dash;
        return segment;
      })
    : null;

  return (
    <div className="donut-wrap">
      <svg viewBox="0 0 140 140" className="donut" role="img" aria-label="来源分布">
        <circle r={radius} cx={cx} cy={cy} fill="none" stroke="var(--border)" strokeWidth={stroke} />
        {segments}
      </svg>
      <div className="donut-center">
        <span className="donut-total">{total}</span>
        <span className="donut-label">总计</span>
      </div>
    </div>
  );
}

/** 标签 Top10：水平条形，颜色用标签自身颜色，缺失时回退调色板。整行可点击下钻。 */
function TagBars({
  data,
  onDrillTag
}: {
  data: TagCountStat[];
  onDrillTag?: (tagName: string) => void;
}) {
  const max = Math.max(1, ...data.map((d) => d.count));
  return (
    <div className="tag-bars">
      {data.map((d, i) => (
        <button
          type="button"
          className="tag-bar-row"
          key={d.name}
          onClick={() => onDrillTag?.(d.name)}
          title={`点击按标签「${d.name}」筛选收藏库`}
        >
          <span className="tag-bar-name" title={d.name}>
            {d.name}
          </span>
          <div className="tag-bar-track">
            <div
              className="tag-bar-fill"
              style={{
                width: `${(d.count / max) * 100}%`,
                background: d.color || CHART_PALETTE[i % CHART_PALETTE.length]
              }}
            />
          </div>
          <span className="tag-bar-count">{d.count}</span>
        </button>
      ))}
    </div>
  );
}

/** 收藏时间线：按月竖向条形。 */
function Timeline({ data }: { data: MonthCount[] }) {
  const max = Math.max(1, ...data.map((d) => d.count));
  return (
    <div className="timeline">
      {data.map((d) => (
        <div className="tl-col" key={d.month} title={`${d.month}: ${d.count}`}>
          <div className="tl-bar-wrap">
            <div className="tl-bar" style={{ height: `${(d.count / max) * 100}%` }} />
          </div>
          <span className="tl-label">{d.month.slice(2)}</span>
        </div>
      ))}
    </div>
  );
}

export function StatsPage({
  refreshToken,
  isActive,
  onDrillSource,
  onDrillTag
}: {
  refreshToken: number;
  isActive: boolean;
  onDrillSource?: (source: string) => void;
  onDrillTag?: (tagName: string) => void;
}) {
  const [stats, setStats] = useState<CollectionStats | null>(null);
  const [loading, setLoading] = useState(true);
  const [range, setRange] = useState(0);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setStats(await api.getCollectionStats(range));
    } catch {
      setStats(null);
    } finally {
      setLoading(false);
    }
  }, [range]);

  useEffect(() => {
    if (isActive) void load();
  }, [isActive, refreshToken, load]);

  const rangeLabel = RANGE_OPTIONS.find((o) => o.days === range)?.label ?? "全部";

  if (loading) {
    return (
      <div className="stats-page">
        <div className="stats-empty">加载中…</div>
      </div>
    );
  }

  if (!stats || stats.total === 0) {
    return (
      <div className="stats-page">
        <div className="stats-empty">暂无收藏数据</div>
      </div>
    );
  }

  return (
    <div className="stats-page">
      <header className="stats-header">
        <div>
          <h2>收藏统计</h2>
          <p>了解你的收藏构成与习惯（仅基于本地元数据）</p>
        </div>
        <div className="stats-range" role="group" aria-label="时间范围">
          {RANGE_OPTIONS.map((o) => (
            <button
              type="button"
              key={o.days}
              className={`range-chip ${range === o.days ? "is-active" : ""}`}
              onClick={() => setRange(o.days)}
            >
              {o.label}
            </button>
          ))}
        </div>
      </header>

      {range !== 0 && (
        <p className="stats-range-note">以下统计已限定在「{rangeLabel}」内（{stats.total} 条）</p>
      )}

      <section className="stats-cards">
        <div className="stat-card">
          <span className="stat-num">{stats.total}</span>
          <span className="stat-label">{rangeLabel}收藏</span>
        </div>
        <div className="stat-card">
          <span className="stat-num">{stats.bySource.length}</span>
          <span className="stat-label">来源数</span>
        </div>
        <div className="stat-card">
          <span className="stat-num">{stats.starredCount}</span>
          <span className="stat-label">已星标</span>
        </div>
        <div className="stat-card">
          <span className="stat-num">{stats.untaggedCount}</span>
          <span className="stat-label">未打标签</span>
        </div>
      </section>

      <div className="stats-grid">
        <section className="stats-panel">
          <h3>
            来源分布
            <span className="panel-hint">点击下钻</span>
          </h3>
          <div className="panel-body source-panel">
            <SourceDonut data={stats.bySource} onDrillSource={onDrillSource} />
            <ul className="legend">
              {stats.bySource.map((d, i) => (
                <li
                  key={d.source}
                  className="legend-item"
                  onClick={() => onDrillSource?.(d.source)}
                  title={`点击按「${d.source}」筛选收藏库`}
                >
                  <span
                    className="legend-dot"
                    style={{ background: CHART_PALETTE[i % CHART_PALETTE.length] }}
                  />
                  <span className="legend-name">{d.source}</span>
                  <span className="legend-count">{d.count}</span>
                </li>
              ))}
            </ul>
          </div>
        </section>

        <section className="stats-panel">
          <h3>
            标签 Top 10
            <span className="panel-hint">点击下钻</span>
          </h3>
          <div className="panel-body">
            {stats.byTag.length === 0 ? (
              <div className="stats-empty small">还没有标签</div>
            ) : (
              <TagBars data={stats.byTag} onDrillTag={onDrillTag} />
            )}
          </div>
        </section>

        <section className="stats-panel stats-panel-wide">
          <h3>收藏时间线（按月）</h3>
          <div className="panel-body">
            {stats.byMonth.length === 0 ? (
              <div className="stats-empty small">暂无时间数据</div>
            ) : (
              <Timeline data={stats.byMonth} />
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
