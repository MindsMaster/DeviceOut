function tzLoad(key, fallback) {
  try {
    var v = localStorage.getItem(key);
    return v === null ? fallback : v;
  } catch (e) {
    return fallback;
  }
}

function tzSave(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (e) {
  }
}

function tzOffset() {
  var v = tzLoad('tz', 'auto');
  if (v === 'auto') return -new Date().getTimezoneOffset();
  var n = parseInt(v, 10);
  return isNaN(n) ? 0 : n;
}

function fmtTs(ms) {
  var n = Number(ms);
  if (!n) return '-';
  return new Date(n + tzOffset() * 60000).toISOString().slice(0, 16).replace('T', ' ');
}

function paintTs() {
  var nodes = document.querySelectorAll('.ts[data-ts]');
  for (var i = 0; i < nodes.length; i++) {
    nodes[i].textContent = fmtTs(nodes[i].getAttribute('data-ts'));
  }
}

paintTs();
