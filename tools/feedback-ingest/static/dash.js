var pal = ['#3b82f6', '#8b5cf6', '#22c55e', '#f59e0b', '#ef4444', '#06b6d4', '#ec4899', '#a3a3a3', '#14b8a6', '#f97316', '#64748b'];
var charts = {};

function setText(id, v) {
  var el = document.getElementById(id);
  if (el) el.textContent = Number(v).toLocaleString();
}

function setPair(chart, d) {
  if (!chart || !d) return;
  chart.data.labels = d.labels || [];
  chart.data.datasets[0].data = d.values || [];
  chart.update('none');
}

function pill(kind) {
  return kind === 'feature'
    ? '<span class="pill feature">建议</span>'
    : '<span class="pill bug">问题</span>';
}

function esc(s) {
  return String(s == null ? '' : s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function renderRows(rows) {
  var body = document.getElementById('ticket-body');
  if (!body) return;
  if (!rows || !rows.length) {
    body.innerHTML = '<tr><td colspan="8" class="empty">无</td></tr>';
    return;
  }
  body.innerHTML = rows.map(function (r) {
    var ver = r.version ? esc(r.version) : '-';
    var contact = r.contact ? esc(r.contact) : '-';
    return '<tr><td class="mono"><a href="' + esc(PREFIX) + '/' + esc(r.ticket) + '">' + esc(r.ticket) + '</a></td>' +
      '<td>' + pill(r.kind) + '</td><td class="muted">' + ver + '</td>' +
      '<td class="muted">' + esc(r.time) + '</td><td class="mono">' + esc(r.ip) + '</td>' +
      '<td>' + contact + '</td><td><span class="excerpt" title="' + esc(r.message) + '">' + esc(r.preview) + '</span></td>' +
      '<td><form class="del" method="post" action="' + esc(PREFIX) + '/delete" onsubmit="return confirm(\'删除？\')">' +
      '<input type="hidden" name="ticket" value="' + esc(r.ticket) + '">' +
      '<button type="submit" class="delbtn">删除</button></form></td></tr>';
  }).join('');
}

function apply(d) {
  setText('n-users', d.users);
  setText('n-online', d.online);
  setText('n-today', d.today);
  setText('n-tickets', d.tickets);
  setText('n-live', d.online);
  setPair(charts.trend, d.trend);
  setPair(charts.ver, d.versions);
  setPair(charts.os, d.os);
  setPair(charts.reg, d.regions);
  setPair(charts.loc, d.locales);
  renderRows(d.rows);
}

function poll() {
  fetch(PREFIX + '/data', { credentials: 'same-origin' })
    .then(function (r) { return r.ok ? r.json() : Promise.reject(); })
    .then(apply)
    .catch(function () {});
}

if (window.Chart) {
  Chart.defaults.color = '#a1a1aa';
  Chart.defaults.borderColor = 'rgba(255,255,255,.06)';
  Chart.defaults.font.family = 'system-ui,-apple-system,"Segoe UI","PingFang SC","Microsoft YaHei",sans-serif';
  var tc = document.getElementById('trend');
  if (tc) {
    var g = tc.getContext('2d').createLinearGradient(0, 0, 0, 230);
    g.addColorStop(0, 'rgba(59,130,246,.35)');
    g.addColorStop(1, 'rgba(59,130,246,0)');
    charts.trend = new Chart(tc, {
      type: 'line',
      data: { labels: DATA.trend.labels, datasets: [{ data: DATA.trend.values, borderColor: '#3b82f6', backgroundColor: g, fill: true, tension: .35, pointRadius: 0, pointHitRadius: 12, borderWidth: 2 }] },
      options: { maintainAspectRatio: false, plugins: { legend: { display: false }, tooltip: { displayColors: false } }, interaction: { intersect: false, mode: 'index' }, scales: { x: { ticks: { maxTicksLimit: 8 }, grid: { display: false } }, y: { beginAtZero: true, ticks: { precision: 0 } } } }
    });
  }
  function dough(id, d) {
    var el = document.getElementById(id);
    if (!el) return null;
    return new Chart(el, {
      type: 'doughnut',
      data: { labels: d.labels, datasets: [{ data: d.values, backgroundColor: pal, borderColor: '#101014', borderWidth: 2, hoverOffset: 4 }] },
      options: { maintainAspectRatio: false, cutout: '64%', plugins: { legend: { position: 'right', labels: { boxWidth: 10, padding: 8, font: { size: 11 } } } } }
    });
  }
  charts.ver = dough('ver', DATA.versions);
  charts.os = dough('os', DATA.os);
  charts.reg = dough('reg', DATA.regions);
  var lc = document.getElementById('loc');
  if (lc) {
    charts.loc = new Chart(lc, {
      type: 'bar',
      data: { labels: DATA.locales.labels, datasets: [{ data: DATA.locales.values, backgroundColor: '#3b82f6', borderRadius: 4, barThickness: 12 }] },
      options: { indexAxis: 'y', maintainAspectRatio: false, plugins: { legend: { display: false } }, scales: { x: { beginAtZero: true, ticks: { precision: 0 } }, y: { grid: { display: false } } } }
    });
  }
}
renderRows(DATA.rows);
setInterval(poll, 8000);
