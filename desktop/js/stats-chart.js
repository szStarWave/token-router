import * as echarts from 'echarts';

const MAX_POINTS = 60;
const historyByScope = { session: [], global: [] };
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

function edgeShade(base, alpha) {
  return base.startsWith('hsl') ? base.replace(')', `, ${alpha})`).replace('hsl', 'hsla') : base;
}

function formatTimeLabel(ts) {
  const d = new Date(ts);
  const h = String(d.getHours()).padStart(2, '0');
  const m = String(d.getMinutes()).padStart(2, '0');
  const s = String(d.getSeconds()).padStart(2, '0');
  return `${h}:${m}:${s}`;
}

function formatAxisNum(n) {
  if (n >= 1e6) return (n / 1e6).toFixed(1) + 'M';
  if (n >= 1e3) return (n / 1e3).toFixed(1) + 'K';
  return String(n);
}

function buildOption(history, labels) {
  const colors = themeColors();
  const times = history.map((p) => formatTimeLabel(p.t));
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
    stack: 'Total',
    smooth: true,
    showSymbol: false,
    emphasis: { focus: 'series' },
    lineStyle: { width: 1, color },
    itemStyle: { color },
    areaStyle: { color: edgeShade(color, alpha) },
    data,
  });

  return {
    animationDuration: 400,
    tooltip: {
      trigger: 'axis',
      axisPointer: { type: 'cross', label: { backgroundColor: '#6a7985' } },
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
      axisLine: { show: false },
      axisLabel: { color: colors.axis, formatter: formatAxisNum },
      splitLine: { lineStyle: { color: colors.split } },
    },
    series: [
      makeSeries(labels.edgeIn, seriesData.edgeIn, colors.edge, 0.55),
      makeSeries(labels.edgeOut, seriesData.edgeOut, colors.edge, 0.35),
      makeSeries(labels.cloudIn, seriesData.cloudIn, colors.cloud, 0.55),
      makeSeries(labels.cloudOut, seriesData.cloudOut, colors.cloud, 0.35),
    ],
  };
}

function renderChart(scope) {
  if (!chart) return;
  const history = historyByScope[scope] || [];
  if (!history.length) {
    chart.clear();
    return;
  }
  chart.setOption(buildOption(history, labelFn()), { notMerge: true });
}

export function installStatsChartGlobals() {
  const el = document.getElementById('token-chart');
  if (!el) return;

  chart = echarts.init(el);
  window.addEventListener('resize', () => chart?.resize());

  window.__statsChart = {
    setLabelFn(fn) {
      labelFn = fn;
    },
    record(scope, tb) {
      if (!scope) return;
      const row = {
        t: Date.now(),
        edgeIn: tb.edge?.input ?? 0,
        edgeOut: tb.edge?.output ?? 0,
        cloudIn: tb.cloud?.input ?? 0,
        cloudOut: tb.cloud?.output ?? 0,
      };
      const hist = historyByScope[scope] || [];
      hist.push(row);
      if (hist.length > MAX_POINTS) hist.shift();
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
      chart?.resize();
    },
    clear(scope) {
      if (scope) historyByScope[scope] = [];
      else historyByScope.session = [];
      historyByScope.global = [];
    },
  };
}
