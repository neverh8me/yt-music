// Applies the saved theme before first paint (external file: CSP forbids inline scripts).
// Themes: "classic" (default) and "glass". Each is a full stylesheet; only one is enabled.
(function () {
  var theme = 'classic';
  try { theme = localStorage.getItem('theme') || 'classic'; } catch (e) { /* storage unavailable */ }
  if (theme !== 'glass') theme = 'classic';
  document.documentElement.dataset.theme = theme;
  document.getElementById('css-classic').disabled = theme !== 'classic';
  document.getElementById('css-glass').disabled = theme !== 'glass';
})();
