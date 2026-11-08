// Applies theme and motion settings before first paint (external file: CSP forbids inline scripts).
// One look (glass, PR #1 by @dopedasya) in three modes: "system" (default), "dark", "light".
// Animations are on unless the user picks "like the system" and the OS asks for less motion.
(function () {
  var get = function (k, d) { try { return localStorage.getItem(k) || d; } catch (e) { return d; } };
  var root = document.documentElement;
  var darkQ = window.matchMedia('(prefers-color-scheme: dark)');
  var lessQ = window.matchMedia('(prefers-reduced-motion: reduce)');
  function apply() {
    var mode = get('themeMode', 'system');
    root.dataset.theme = mode === 'light' || (mode === 'system' && !darkQ.matches) ? 'light' : 'dark';
    root.dataset.motion = get('motion', 'on') === 'system' && lessQ.matches ? 'reduce' : 'full';
  }
  apply();
  window.__applyTheme = apply;
  if (darkQ.addEventListener) { darkQ.addEventListener('change', apply); lessQ.addEventListener('change', apply); }
})();
