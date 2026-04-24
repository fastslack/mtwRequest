/* ══════════════════════════════════════════════════════════════════════
   mtwRequest · benchmark report · client-side
   — Count-up numbers on scroll
   — ApexCharts configs (fanout / echo / connect / journey)
   — Fanout 50/500 toggle + linear/log scale toggle
   — Bar reveal animations
   ══════════════════════════════════════════════════════════════════════ */

'use strict';

// ─── Colour tokens (synced with CSS custom props) ───────────────────────
const COLOR = {
  mtw:  '#2DE5B8',
  nats: '#F4B860',
  cent: '#B098F7',
  sio:  '#ED6F82',
  ink:  '#EDEFF3',
  ink2: '#B7BCC8',
  ink3: '#7B8294',
  ink4: '#4D5362',
  surf: '#12141A',
  brd:  '#1C1F28',
};

const SYS = [
  { key: 'mtw',  name: 'mtwRequest', color: COLOR.mtw  },
  { key: 'nats', name: 'NATS',       color: COLOR.nats },
  { key: 'cent', name: 'Centrifugo', color: COLOR.cent },
  { key: 'sio',  name: 'Socket.IO',  color: COLOR.sio  },
];

// ─── Ground-truth benchmark data (2026-04-23 · mtw w/ jemalloc) ─────────
const DATA = {
  fanout: {
    50: {
      p50: { mtw: 4.80,  nats: 15.96, cent: 18.87, sio: 186.91 },
      p99: { mtw: 9.94,  nats: 18.42, cent: 28.77, sio: 361.50 },
      tp:  { mtw: 25.2,  nats: 89.0,  cent: 22.0,  sio: 15.7 },   // M msg/s
    },
    500: {
      p50: { mtw: 52.46, nats: 55.15,  cent: 148.24, sio: 2130.00 },
      p99: { mtw: 85.26, nats: 91.49,  cent: 251.40, sio: 3810.00 },
      tp:  { mtw: 249.3, nats: 1329.2, cent: 148.8,  sio: 11.9 },
    },
  },
  echo: {
    p50: { mtw: 58.0,  nats: 58.4,  cent: 63.4,  sio: 113.7 }, // µs
    p99: { mtw: 101.6, nats: 93.3,  cent: 123.9, sio: 189.1 },
  },
  connect: {
    rate: { mtw: 48600, nats: 18100, cent: 24800, sio: 182 }, // /s
    p50:  { mtw: 0.529, nats: 1.610, cent: 1.340, sio: 158.99 }, // ms
    p99:  { mtw: 0.953, nats: 3.580, cent: 1.750, sio: 181.67 },
  },
  journey: [
    { v: 'v0', ms: 418.9, label: 'baseline',           desc: 'Pre-optimization. Straight tungstenite + default writer loop.' },
    { v: 'v1', ms: 282.3, label: 'encode-once',        desc: 'SharedEnvelope lazily caches text + binary wire bytes. Fanout now serializes once, not N times.' },
    { v: 'v2', ms: 130.7, label: 'direct sink',        desc: 'Publish delivers straight into each conn writer queue. Subscriber list becomes a lock-free ArcSwap snapshot.' },
    { v: 'v3', ms: 169.0, label: 'ConnTarget cache',   desc: 'Each subscriber resolves its writer once at subscribe time. Zero DashMap lookups on the hot path. (Apparent regression was run-to-run noise.)' },
    { v: 'v4', ms: 107.0, label: 'write batching',     desc: 'Writer task feeds many frames, flushes once per burst. TCP syscalls drop ~10×.' },
    { v: 'v5', ms:  64.5, label: 'zero-copy text',     desc: 'tokio-tungstenite 0.29 + Utf8Bytes::from_bytes_unchecked. One fewer heap alloc per delivery.' },
  ],
};

// ─── Chart.js-free: shared Apex defaults for this dark theme ────────────
const apexBase = {
  chart: {
    background: 'transparent',
    toolbar: { show: false },
    animations: {
      // Keep off for reliable first-render in any browser / headless env.
      // Hover + update animations still feel responsive via `dynamicAnimation`.
      enabled: false,
      dynamicAnimation: { enabled: true, speed: 300 },
    },
    fontFamily: '"Geist", system-ui, sans-serif',
  },
  grid: {
    borderColor: COLOR.brd,
    strokeDashArray: 2,
    xaxis: { lines: { show: false } },
    yaxis: { lines: { show: true } },
    padding: { left: 12, right: 12, top: 0, bottom: 0 },
  },
  tooltip: {
    theme: 'dark',
    style: { fontFamily: '"JetBrains Mono", monospace', fontSize: '12px' },
    marker: { show: true },
    x: { show: true },
  },
  legend: { show: false },
  dataLabels: { enabled: false },
  stroke: { curve: 'smooth' },
};

// ─── Count-up numbers on reveal ─────────────────────────────────────────
function countUp(el) {
  const target  = parseFloat(el.dataset.count);
  const decimals = parseInt(el.dataset.decimals || '0', 10);
  const suffix  = el.dataset.suffix || '';
  const duration = 1600;
  const start = performance.now();

  function tick(now) {
    const t = Math.min(1, (now - start) / duration);
    // easeOutExpo
    const eased = t === 1 ? 1 : 1 - Math.pow(2, -10 * t);
    const val = target * eased;
    el.textContent = val.toFixed(decimals) + suffix;
    if (t < 1) requestAnimationFrame(tick);
    else el.textContent = target.toFixed(decimals) + suffix;
  }
  requestAnimationFrame(tick);
}

function initCountUps() {
  const els = document.querySelectorAll('[data-count]');
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting && !entry.target.dataset.done) {
        entry.target.dataset.done = '1';
        countUp(entry.target);
      }
    });
  }, { threshold: 0.4 });
  els.forEach((el) => observer.observe(el));
}

// ─── Throughput bar reveal ──────────────────────────────────────────────
function initBarReveals() {
  const bars = document.querySelectorAll('.cb-fill');
  const observer = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add('animate');
        observer.unobserve(entry.target);
      }
    });
  }, { threshold: 0.3 });
  bars.forEach((b) => observer.observe(b));
}

// ─── Fanout chart ──────────────────────────────────────────────────────
let fanoutChart = null;
let currentSubs = 50;
let currentScale = 'linear';

function buildFanoutOptions(subs, scale) {
  const src = DATA.fanout[subs];
  const useLog = scale === 'log';
  const fmt = (v) => v >= 1000 ? (v / 1000).toFixed(2) + ' s'
                  : v >= 1    ? v.toFixed(2) + ' ms'
                              : (v * 1000).toFixed(0) + ' µs';

  // Each system is a series so ApexCharts colors each group consistently.
  const series = SYS.map((s) => ({
    name: s.name,
    data: [src.p50[s.key], src.p99[s.key]],
  }));

  return {
    ...apexBase,
    chart: {
      ...apexBase.chart,
      type: 'bar',
      height: 420,
    },
    series,
    colors: SYS.map((s) => s.color),
    plotOptions: {
      bar: {
        borderRadius: 4,
        borderRadiusApplication: 'end',
        columnWidth: '68%',
        distributed: false,
        dataLabels: { position: 'top' },
      },
    },
    fill: { type: 'solid', opacity: 1 },
    stroke: { show: false },
    dataLabels: {
      enabled: true,
      offsetY: -24,
      style: {
        fontSize: '10px',
        fontFamily: '"JetBrains Mono", monospace',
        colors: [COLOR.ink2],
        fontWeight: 500,
      },
      formatter: (val) => fmt(val),
    },
    xaxis: {
      categories: ['p50', 'p99'],
      labels: {
        style: {
          colors: COLOR.ink2,
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: '13px',
          fontWeight: 600,
        },
        offsetY: 4,
      },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      logarithmic: useLog,
      logBase: 10,
      forceNiceScale: !useLog,
      labels: {
        style: { colors: COLOR.ink3, fontFamily: '"JetBrains Mono", monospace', fontSize: '11px' },
        formatter: (val) => {
          if (val >= 1000) return (val / 1000).toFixed(1) + 's';
          if (val >= 1)    return val.toFixed(0) + 'ms';
          return val.toFixed(2) + 'ms';
        },
      },
      title: {
        text: useLog ? 'Latency — log scale' : 'Latency',
        style: { color: COLOR.ink4, fontFamily: '"JetBrains Mono", monospace', fontWeight: 400, fontSize: '11px' },
      },
      axisBorder: { show: false },
    },
    tooltip: {
      ...apexBase.tooltip,
      shared: false,
      intersect: true,
      custom: ({ seriesIndex, dataPointIndex }) => {
        const sys = SYS[seriesIndex];
        const src = DATA.fanout[currentSubs];
        const p50 = src.p50[sys.key];
        const p99 = src.p99[sys.key];
        const tp  = src.tp[sys.key];
        return `
          <div class="apexcharts-tooltip-title" style="border-left: 3px solid ${sys.color}">${sys.name}</div>
          <div style="padding: 10px 14px 12px; font-family: 'JetBrains Mono', monospace; font-size: 11px; line-height: 1.7">
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p50</span>
              <span style="color:${COLOR.ink}; font-weight:600">${fmt(p50)}</span>
            </div>
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p99</span>
              <span style="color:${COLOR.ink}">${fmt(p99)}</span>
            </div>
            <div style="display:flex; justify-content:space-between; gap:30px; margin-top:4px; padding-top:6px; border-top:1px dashed ${COLOR.brd}">
              <span style="color:${COLOR.ink4}">throughput</span>
              <span style="color:${COLOR.ink2}">${tp >= 1000 ? (tp/1000).toFixed(2)+' G/s' : tp.toFixed(1)+' M/s'}</span>
            </div>
          </div>`;
      },
    },
    grid: {
      ...apexBase.grid,
      padding: { left: 12, right: 12, top: 24, bottom: 0 },
    },
  };
}

function renderFanout() {
  const opts = buildFanoutOptions(currentSubs, currentScale);
  if (!fanoutChart) {
    fanoutChart = new ApexCharts(document.querySelector('#chart-fanout'), opts);
    fanoutChart.render();
  } else {
    fanoutChart.updateOptions(opts, false, true);
  }
  // Update chart title
  const titleEl = document.getElementById('fanout-title');
  if (titleEl) {
    titleEl.textContent = `p50 & p99 latency · ${currentSubs} subscribers · lower is better`;
  }
  // Hide footnote when 50, show when 500
  const fn = document.getElementById('fanout-footnote');
  if (fn) {
    fn.style.display = currentSubs === 500 ? 'block' : 'none';
  }
}

function initFanoutToggles() {
  document.querySelectorAll('.toggle').forEach((btn) => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.toggle').forEach((b) => {
        b.classList.remove('active');
        b.setAttribute('aria-selected', 'false');
      });
      btn.classList.add('active');
      btn.setAttribute('aria-selected', 'true');
      currentSubs = parseInt(btn.dataset.subs, 10);
      renderFanout();
    });
  });

  document.querySelectorAll('.scale').forEach((btn) => {
    btn.addEventListener('click', () => {
      document.querySelectorAll('.scale').forEach((b) => {
        b.classList.remove('active');
        b.setAttribute('aria-selected', 'false');
      });
      btn.classList.add('active');
      btn.setAttribute('aria-selected', 'true');
      currentScale = btn.dataset.scale;
      renderFanout();
    });
  });
}

// ─── Echo chart ────────────────────────────────────────────────────────
function renderEcho() {
  // Each system = own series so ApexCharts applies per-system color.
  const series = SYS.map((s) => ({
    name: s.name,
    data: [DATA.echo.p50[s.key], DATA.echo.p99[s.key]],
  }));

  const opts = {
    ...apexBase,
    chart: { ...apexBase.chart, type: 'bar', height: 340 },
    series,
    colors: SYS.map((s) => s.color),
    plotOptions: {
      bar: { borderRadius: 3, columnWidth: '68%', borderRadiusApplication: 'end' },
    },
    fill: { type: 'solid', opacity: 1 },
    stroke: { show: false },
    dataLabels: {
      enabled: true,
      offsetY: -22,
      style: {
        fontSize: '10px',
        fontFamily: '"JetBrains Mono", monospace',
        colors: [COLOR.ink2],
        fontWeight: 500,
      },
      formatter: (val) => val.toFixed(0) + 'µs',
    },
    xaxis: {
      categories: ['p50', 'p99'],
      labels: {
        style: {
          colors: COLOR.ink2,
          fontFamily: '"JetBrains Mono", monospace',
          fontSize: '12px',
          fontWeight: 600,
        },
        offsetY: 4,
      },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      labels: {
        style: { colors: COLOR.ink3, fontFamily: '"JetBrains Mono", monospace', fontSize: '11px' },
        formatter: (v) => v.toFixed(0) + ' µs',
      },
      title: {
        text: 'Round-trip latency',
        style: { color: COLOR.ink4, fontFamily: '"JetBrains Mono", monospace', fontWeight: 400, fontSize: '11px' },
      },
    },
    tooltip: {
      ...apexBase.tooltip,
      shared: false,
      intersect: true,
      custom: ({ seriesIndex }) => {
        const sys = SYS[seriesIndex];
        const p50v = DATA.echo.p50[sys.key];
        const p99v = DATA.echo.p99[sys.key];
        return `
          <div class="apexcharts-tooltip-title" style="border-left: 3px solid ${sys.color}">${sys.name}</div>
          <div style="padding: 10px 14px 12px; font-family: 'JetBrains Mono', monospace; font-size: 11px; line-height: 1.7">
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p50</span>
              <span style="color:${COLOR.ink}; font-weight:600">${p50v} µs</span>
            </div>
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p99</span>
              <span style="color:${COLOR.ink}">${p99v} µs</span>
            </div>
          </div>`;
      },
    },
    grid: { ...apexBase.grid, padding: { left: 12, right: 12, top: 24, bottom: 0 } },
  };
  new ApexCharts(document.querySelector('#chart-echo'), opts).render();
}

// ─── Connect chart ─────────────────────────────────────────────────────
function renderConnect() {
  const rate = SYS.map((s) => DATA.connect.rate[s.key]);

  const opts = {
    ...apexBase,
    chart: { ...apexBase.chart, type: 'bar', height: 340 },
    series: [
      { name: 'connects/s', data: SYS.map((s, i) => ({ x: s.name, y: rate[i], fillColor: s.color })) },
    ],
    plotOptions: {
      bar: { borderRadius: 4, columnWidth: '54%', borderRadiusApplication: 'end', horizontal: false, distributed: true },
    },
    fill: { type: 'solid', opacity: 1 },
    colors: SYS.map((s) => s.color),
    dataLabels: {
      enabled: true,
      offsetY: -22,
      style: {
        fontSize: '11px',
        fontFamily: '"JetBrains Mono", monospace',
        colors: [COLOR.ink2],
        fontWeight: 500,
      },
      formatter: (val) => {
        if (val >= 1000) return (val / 1000).toFixed(1) + ' k/s';
        return val + ' /s';
      },
    },
    xaxis: {
      labels: { style: { colors: COLOR.ink3, fontFamily: '"Geist", sans-serif', fontSize: '12px' } },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      logarithmic: true,
      logBase: 10,
      labels: {
        style: { colors: COLOR.ink3, fontFamily: '"JetBrains Mono", monospace', fontSize: '11px' },
        formatter: (v) => {
          if (v >= 1000) return (v / 1000).toFixed(0) + ' k';
          return v.toFixed(0);
        },
      },
      title: {
        text: 'Handshakes / second (log scale)',
        style: { color: COLOR.ink4, fontFamily: '"JetBrains Mono", monospace', fontWeight: 400, fontSize: '11px' },
      },
    },
    tooltip: {
      ...apexBase.tooltip,
      custom: ({ dataPointIndex }) => {
        const sys = SYS[dataPointIndex];
        const rateV = DATA.connect.rate[sys.key];
        const p50 = DATA.connect.p50[sys.key];
        const p99 = DATA.connect.p99[sys.key];
        const fmtRate = (v) => v >= 1000 ? (v / 1000).toFixed(1) + ' k/s' : v.toFixed(0) + ' /s';
        const fmtMs = (v) => v >= 1 ? v.toFixed(2) + ' ms' : (v * 1000).toFixed(0) + ' µs';
        return `
          <div class="apexcharts-tooltip-title" style="border-left: 3px solid ${sys.color}">${sys.name}</div>
          <div style="padding: 10px 14px 12px; font-family: 'JetBrains Mono', monospace; font-size: 11px; line-height: 1.7">
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">rate</span>
              <span style="color:${COLOR.ink}; font-weight:600">${fmtRate(rateV)}</span>
            </div>
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p50 connect</span>
              <span style="color:${COLOR.ink2}">${fmtMs(p50)}</span>
            </div>
            <div style="display:flex; justify-content:space-between; gap:30px">
              <span style="color:${COLOR.ink3}">p99 connect</span>
              <span style="color:${COLOR.ink2}">${fmtMs(p99)}</span>
            </div>
          </div>`;
      },
    },
    grid: { ...apexBase.grid, padding: { left: 12, right: 12, top: 24, bottom: 0 } },
  };
  new ApexCharts(document.querySelector('#chart-connect'), opts).render();
}

// ─── Journey chart ─────────────────────────────────────────────────────
function renderJourney() {
  const ms = DATA.journey.map((j) => j.ms);
  const labels = DATA.journey.map((j) => j.v);

  const opts = {
    ...apexBase,
    chart: {
      ...apexBase.chart,
      type: 'area',
      height: 380,
      sparkline: { enabled: false },
    },
    series: [{ name: 'p50 latency (ms)', data: ms }],
    colors: [COLOR.mtw],
    stroke: {
      curve: 'smooth',
      width: 3,
      colors: [COLOR.mtw],
    },
    fill: {
      type: 'gradient',
      gradient: {
        shadeIntensity: 1,
        opacityFrom: 0.35,
        opacityTo: 0.02,
        stops: [0, 90, 100],
        colorStops: [
          { offset: 0, color: COLOR.mtw, opacity: 0.35 },
          { offset: 100, color: COLOR.mtw, opacity: 0.02 },
        ],
      },
    },
    markers: {
      size: 6,
      strokeWidth: 2,
      strokeColors: COLOR.surf,
      colors: [COLOR.mtw],
      hover: { size: 10, sizeOffset: 3 },
    },
    dataLabels: {
      enabled: true,
      offsetY: -12,
      background: {
        enabled: true,
        foreColor: COLOR.ink,
        padding: 4,
        borderRadius: 4,
        borderWidth: 1,
        borderColor: COLOR.brd,
        opacity: 1,
        dropShadow: { enabled: false },
      },
      style: {
        fontSize: '10px',
        fontFamily: '"JetBrains Mono", monospace',
        fontWeight: 500,
        colors: [COLOR.ink],
      },
      formatter: (val) => val.toFixed(1) + ' ms',
    },
    xaxis: {
      categories: labels,
      labels: {
        style: { colors: COLOR.ink3, fontFamily: '"JetBrains Mono", monospace', fontSize: '12px', fontWeight: 600 },
      },
      axisBorder: { show: false },
      axisTicks: { show: false },
    },
    yaxis: {
      labels: {
        style: { colors: COLOR.ink3, fontFamily: '"JetBrains Mono", monospace', fontSize: '11px' },
        formatter: (v) => v.toFixed(0) + ' ms',
      },
      title: {
        text: 'Fanout 500 subs · p50',
        style: { color: COLOR.ink4, fontFamily: '"JetBrains Mono", monospace', fontWeight: 400, fontSize: '11px' },
      },
    },
    tooltip: {
      ...apexBase.tooltip,
      custom: ({ dataPointIndex }) => {
        const j = DATA.journey[dataPointIndex];
        return `
          <div class="apexcharts-tooltip-title" style="border-left: 3px solid ${COLOR.mtw}">${j.v} · ${j.label}</div>
          <div style="padding: 10px 14px 14px; font-family: 'Geist', sans-serif; font-size: 12px; line-height: 1.55; max-width: 320px">
            <div style="font-family: 'JetBrains Mono', monospace; color: ${COLOR.mtw}; font-size: 14px; font-weight: 600; margin-bottom: 6px">${j.ms.toFixed(1)} ms</div>
            <div style="color: ${COLOR.ink2}">${j.desc}</div>
          </div>`;
      },
    },
    grid: {
      ...apexBase.grid,
      padding: { left: 12, right: 12, top: 28, bottom: 0 },
    },
    annotations: {
      points: [
        {
          x: 'v0',
          y: DATA.journey[0].ms,
          marker: { size: 0 },
          label: {
            borderColor: COLOR.brd,
            borderWidth: 1,
            borderRadius: 4,
            offsetY: -14,
            style: {
              color: COLOR.ink3,
              background: COLOR.surf,
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: '10px',
              padding: { left: 6, right: 6, top: 3, bottom: 3 },
            },
            text: 'baseline',
          },
        },
        {
          x: 'v5',
          y: DATA.journey[DATA.journey.length - 1].ms,
          marker: { size: 0 },
          label: {
            borderColor: COLOR.mtw,
            borderWidth: 1,
            borderRadius: 4,
            offsetY: 30,
            style: {
              color: COLOR.mtw,
              background: 'rgba(45,229,184,0.08)',
              fontFamily: '"JetBrains Mono", monospace',
              fontSize: '10px',
              fontWeight: 600,
              padding: { left: 6, right: 6, top: 3, bottom: 3 },
            },
            text: 'shipped',
          },
        },
      ],
    },
  };
  new ApexCharts(document.querySelector('#chart-journey'), opts).render();
}

// ─── Boot ──────────────────────────────────────────────────────────────
function boot() {
  if (typeof ApexCharts === 'undefined') {
    console.error('ApexCharts failed to load.');
    return;
  }
  initCountUps();
  initBarReveals();
  initFanoutToggles();

  // Render the charts. Use IntersectionObserver to defer the heavier ones
  // (not critical above the fold).
  renderFanout();
  renderEcho();
  renderConnect();
  renderJourney();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
