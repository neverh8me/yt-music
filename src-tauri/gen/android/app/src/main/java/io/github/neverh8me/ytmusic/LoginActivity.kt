package io.github.neverh8me.ytmusic

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.webkit.WebSettingsCompat
import androidx.webkit.WebViewFeature

/**
 * "Sign in with Google" on Android: Google's own sign-in page in a WebView. Once it lands
 * on YouTube with a signed-in session (SAPISID cookie), the YouTube cookies are handed back
 * to MainActivity, which gives them to Rust (same path as the desktop sign-in window).
 * The WebView's cookies are wiped afterwards, so no Google session stays in the app.
 */
class LoginActivity : Activity() {
  companion object {
    const val EXTRA_COOKIE = "cookie"
    private const val START =
      "https://accounts.google.com/ServiceLogin?ltmpl=music&service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F"
    // Fallback only: a regular mobile Chrome UA.
    private const val UA_FALLBACK =
      "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36"
  }

  /**
   * The system WebView's own UA without its WebView markers ("; wv", "Version/4.0"), so it reads
   * as mobile Chrome of the *same* version as the engine. Google refuses sign-in in browsers it
   * detects as embedded, and also when the claimed Chrome version doesn't match what the engine
   * can do ("This browser or app may not be secure"), e.g. Chrome 142 claimed on a WebView 92.
   */
  private fun userAgent(): String = try {
    android.webkit.WebSettings.getDefaultUserAgent(this)
      .replace("; wv)", ")")
      .replace(Regex("""\s*Build/[^;)]*"""), "")
      .replace(Regex("""\s*Version/[\d.]+"""), "")
  } catch (e: Exception) {
    UA_FALLBACK
  }

  private val main = Handler(Looper.getMainLooper())
  private var done = false

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val cookies = CookieManager.getInstance()
    cookies.setAcceptCookie(true)
    val web = WebView(this)
    cookies.setAcceptThirdPartyCookies(web, true)
    web.settings.javaScriptEnabled = true
    web.settings.domStorageEnabled = true
    web.settings.userAgentString = userAgent()
    // Don't announce the app package in X-Requested-With (Google uses it to spot WebViews).
    if (WebViewFeature.isFeatureSupported(WebViewFeature.REQUESTED_WITH_HEADER_ALLOW_LIST)) {
      WebSettingsCompat.setRequestedWithHeaderOriginAllowList(web.settings, emptySet())
    }
    web.webViewClient = object : WebViewClient() {
      override fun onPageFinished(view: WebView, url: String) {
        if (url.contains("youtube.com")) poll(0)
      }
    }
    setContentView(web)
    web.loadUrl(START)
  }

  /** The session cookie can show up a moment after the page finishes loading. */
  private fun poll(attempt: Int) {
    if (done || isFinishing) return
    val c = CookieManager.getInstance().getCookie("https://www.youtube.com") ?: ""
    if (c.contains("SAPISID=") || c.contains("__Secure-3PAPISID=")) {
      done = true
      setResult(RESULT_OK, Intent().putExtra(EXTRA_COOKIE, c))
      CookieManager.getInstance().removeAllCookies(null)
      CookieManager.getInstance().flush()
      finish()
    } else if (attempt < 40) {
      main.postDelayed({ poll(attempt + 1) }, 500)
    }
  }
}
