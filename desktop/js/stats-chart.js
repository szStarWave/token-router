import * as echarts from 'echarts';
import { formatAxisNum as formatAxisNumBase } from './format-number.js';

const MAX_MINUTES = 60;
const historyByScope = { session: [], global: [] };
const lastTotalsByScope = { session: null, global: null };
let chart = null;
let labelFn = () => ({
  edgeIn: 'Edge Input',
  edgeOut: 'Edge Output',
  cloudIn: 'Cloud Input',
  cloudOut: 'Cloud Output',
});

function themeColors() {
  const dark = document.documentElement.dataset.effectiveTheme === 'dark';
  return {
    axis: dark ? '#a1a1aa' : '#71717a',
    split: dark ? 'rgba(255,255,255,0.06)' : 'rgba(0,0,0,0.06)',
    edge: getComputedStyle(document.documentElement).getPropertyValue('--edge').trim() || 'hsl(160, 60%, 45%)',
    cloud: getComputedStyle(document.documentElement).getPropertyValue('--cloud').trim() || 'hsl(262, 70%, 58%)',
  };
}

function colorAlpha(base, alpha) {
  return base.startsWith('hsl') ? base.replace(')', `, ${alpha})`).replace('hsl', 'hsla') : base;
}

function minuteKey(ts = Date.now()) {
  return Math.floor(ts / 60000);
}

function formatMinuteLabel(key) {
  const d = new Date(key * 60000);
  const h = String(d.getHours()).padStart(2, '0');
  const m = String(d.getMinutes()).padStart(2, '0');
  return `${h}:${m}`;
}

function chartLocale() {
  return window.__appI18n?.locale?.() || 'zh';
}

function formatAxisNum(n) {
  return formatAxisNumBase(n, chartLocale());
}

function snapshotFromTb(tb) {
  return {
    edgeIn: Number(tb.edge?.input) || 0,
    edgeOut: Number(tb.edge?.output) || 0,
    cloudIn: Number(tb.cloud?.input) || 0,
    cloudOut: Number(tb.cloud?.output) || 0,
  };
}

function countersReset(cur, prev) {
  return cur.edgeIn < prev.edgeIn
    || cur.edgeOut < prev.edgeOut
    || cur.cloudIn < prev.cloudIn
    || cur.cloudOut < prev.cloudOut;
}

function fieldDelta(cur, prev) {
  if (prev == null) return 0;
  return Math.max(0, cur - prev);
}

function deltaFromSnapshots(cur, prev) {
  return {
    edgeIn: fieldDelta(cur.edgeIn, prev.edgeIn),
    edgeOut: fieldDelta(cur.edgeOut, prev.edgeOut),
    cloudIn: fieldDelta(cur.cloudIn, prev.cloudIn),
    cloudOut: fieldDelta(cur.cloudOut, prev.cloudOut),
  };
}

function hasDelta(delta) {
  return delta.edgeIn > 0 || delta.edgeOut > 0 || delta.cloudIn > 0 || delta.cloudOut > 0;
}

function addDeltaToHistory(hist, key, delta) {
  const last = hist[hist.length - 1];
  if (last && last.minute === key) {
    last.edgeIn += delta.edgeIn;
    last.edgeOut += delta.edgeOut;
    last.cloudIn += delta.cloudIn;
    last.cloudOut += delta.cloudOut;
    return;
  }
  hist.push({
    minute: key,
    edgeIn: delta.edgeIn,
    edgeOut: delta.edgeOut,
    cloudIn: delta.cloudIn,
    cloudOut: delta.cloudOut,
  });
  if (hist.length > MAX_MINUTES) hist.shift();
}

function seedCurrentMinute(hist) {
  const key = minuteKey();
  const last = hist[hist.length - 1];
  if (last && last.minute === key) return;
  hist.push({
    minute: key,
    edgeIn: 0,
    edgeOut: 0,
    cloudIn: 0,
    cloudOut: 0,
  });
  if (hist.length > MAX_MINUTES) hist.shift();
}

function ensureChart() {
  const el = document.getElementById('token-chart');
  if (!el) return null;
  if (!chart) chart = echarts.init(el);
  return chart;
}

function buildOption(history, labels) {
  const colors = themeColors();
  const times = history.map((p) => formatMinuteLabel(p.minute));
  const seriesData = {
    edgeIn: history.map((p) => p.edgeIn),
    edgeOut: history.map((p) => p.edgeOut),
    cloudIn: history.map((p) => p.cloudIn),
    cloudOut: history.map((p) => p.cloudOut),
  };
  const legend = [labels.edgeIn, labels.edgeOut, labels.cloudIn, labels.cloudOut];

  const makeSeries = (name, data, color, alpha) => ({
    name,
    type: 'line',
    smooth: true,
    showSymbol: false,
    emphasis: { focus: 'series' },
    lineStyle: { width: 2, color },
    itemStyle: { color },
    areaStyle: { opacity: alpha, color },
    data,
  });

  return {
    animationDuration: 400,
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'cross', label: { backgroundColor: '#6a7985' } },
      valueFormatter: (v) => formatAxisNum(v),
    },
    legend: { data: legend, textStyle: { color: colors.axis }, top: 0 },
    grid: { left: '3%', right: '4%', bottom: '3%', top: 36, containLabel: true },
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: times,
      axisLine: { lineStyle: { color: colors.split } },
      axisLabel: { color: colors.axis, fontSize: 11 },
    },
    yAxis: {
      type: 'value',
      min: 0,
      axisLine: { show: false },
      axisLabel: { color: colors.axis, formatter: formatAxisNum },
      splitLine: { lineStyle: { color: colors.split } },
    },
    series: [
      makeSeries(labels.edgeIn, seriesData.edgeIn, colors.edge, 0.18),
      makeSeries(labels.edgeOut, seriesData.edgeOut, colorAlpha(colors.edge, 0.72), 0.12),
      makeSeries(labels.cloudIn, seriesData.cloudIn, colors.cloud, 0.18),
      makeSeries(labels.cloudOut, seriesData.cloudOut, colorAlpha(colors.cloud, 0.72), 0.12),
    ],
  };
}

function renderChart(scope) {
  const c = ensureChart();
  if (!c) return;
  const history = historyByScope[scope] || [];
  c.resize();
  if (!history.length) {
    c.clear();
    return;
  }
  c.setOption(buildOption(history, labelFn()), { notMerge: true });
}

export function installStatsChartGlobals() {
  if (!document.getElementById('token-chart')) return;

  window.addEventListener('resize', () => chart?.resize());

  window.__statsChart = {
    setLabelFn(fn) {
      labelFn = fn;
    },
    record(scope, tb) {
      if (!scope) return;
      const cur = snapshotFromTb(tb || {});
      const prev = lastTotalsByScope[scope];
      const hist = historyByScope[scope] || [];

      if (prev == null) {
        lastTotalsByScope[scope] = { ...cur };
        seedCurrentMinute(hist);
        historyByScope[scope] = hist;
        return;
      }

      if (countersReset(cur, prev)) {
        lastTotalsByScope[scope] = { ...cur };
        seedCurrentMinute(hist);
        historyByScope[scope] = hist;
        return;
      }

      const delta = deltaFromSnapshots(cur, prev);
      lastTotalsByScope[scope] = { ...cur };
      if (!hasDelta(delta)) return;

      addDeltaToHistory(hist, minuteKey(), delta);
      historyByScope[scope] = hist;
    },
    render(scope) {
      renderChart(scope || 'session');
    },
    onThemeChange() {
      const scope = document.querySelector('.scope-btn.active')?.dataset.scope || 'session';
      renderChart(scope);
    },
    resize() {
      ensureChart()?.resize();
    },
    clear(scope) {
      if (scope) {
        historyByScope[scope] = [];
        lastTotalsByScope[scope] = null;
      } else {
        historyByScope.session = [];
        historyByScope.global = [];
        lastTotalsByScope.session = null;
        lastTotalsByScope.global = null;
      }
    },
  };
}
