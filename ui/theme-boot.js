// Applies the saved theme before first paint (external file: CSP forbids inline scripts).
// Themes: "glass" (default, PR #1 by @dopedasya) and "classic". Both are token sets in styles.css.
(function () {
  var theme = 'glass';
  try { theme = localStorage.getItem('theme') || 'glass'; } catch (e) { /* storage unavailable */ }
  if (theme !== 'classic') theme = 'glass';
  document.documentElement.dataset.theme = theme;
})();
