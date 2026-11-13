package io.github.neverh8me.ytmusic

import android.Manifest
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.os.Build
import android.os.Bundle
import android.content.Intent
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  private val LOGIN_REQUEST = 7

  @Deprecated("Activity result API of the Tauri base activity")
  override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
    super.onActivityResult(requestCode, resultCode, data)
    if (requestCode != LOGIN_REQUEST) return
    val cookie = data?.getStringExtra(LoginActivity.EXTRA_COOKIE)
    val arg = if (resultCode == RESULT_OK && cookie != null) JSONObject.quote(cookie) else "null"
    webView?.evaluateJavascript("window.__androidLogin && window.__androidLogin($arg)", null)
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // Phones: portrait only (the phone UI is built for it, like the official app).
    // Tablets: rotate, but honour the system auto-rotate lock ("USER").
    requestedOrientation =
      if (resources.configuration.smallestScreenWidthDp < 600) ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT
      else ActivityInfo.SCREEN_ORIENTATION_USER
    // Android 13+: the playback notification needs this permission.
    if (Build.VERSION.SDK_INT >= 33 &&
      checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
      requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
    }
    // Lock screen / headset / notification buttons → the web player.
    MediaPlaybackService.onAction = { action, arg ->
      runOnUiThread {
        webView?.evaluateJavascript("window.__androidMedia && window.__androidMedia('$action', $arg)", null)
      }
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    webView.addJavascriptInterface(MediaBridge(), "AndroidMedia")
    webView.addJavascriptInterface(AuthBridge(), "AndroidAuth")
  }

  // The base activity pauses the WebView when the app leaves the screen. The page is the
  // player: paused, it can't start the next song when one ends. Keep it running.
  override fun onPause() {
    super.onPause()
    webView?.onResume()
  }

  override fun onDestroy() {
    MediaPlaybackService.onAction = null
    MediaPlaybackService.stop(this)
    super.onDestroy()
  }

  /** "Sign in with Google" from the web UI: opens LoginActivity. */
  inner class AuthBridge {
    @JavascriptInterface
    fun login() {
      runOnUiThread {
        @Suppress("DEPRECATION")
        startActivityForResult(Intent(this@MainActivity, LoginActivity::class.java), LOGIN_REQUEST)
      }
    }
  }

  /** Called from ui/player.js with the current track and playback state. */
  inner class MediaBridge {
    private val io = Executors.newSingleThreadExecutor()
    private var artUrl: String? = null
    private var art: Bitmap? = null

    @JavascriptInterface
    fun update(json: String) {
      val o = JSONObject(json)
      val cover = o.optString("cover").takeIf { it.startsWith("https://") }
      io.execute {
        if (cover != artUrl) {
          art = cover?.let { loadBitmap(it) }
          artUrl = cover
        }
        MediaPlaybackService.update(
          this@MainActivity,
          MediaPlaybackService.NowPlaying(
            title = o.optString("title"),
            artist = o.optString("artist"),
            album = o.optString("album"),
            art = art,
            durationMs = o.optLong("durationMs"),
            positionMs = o.optLong("positionMs"),
            playing = o.optBoolean("playing"),
          )
        )
      }
    }

    @JavascriptInterface
    fun stop() {
      MediaPlaybackService.stop(this@MainActivity)
    }

    private fun loadBitmap(url: String): Bitmap? = try {
      (URL(url).openConnection() as HttpURLConnection).run {
        connectTimeout = 5000
        readTimeout = 5000
        inputStream.use { BitmapFactory.decodeStream(it) }
      }
    } catch (e: Exception) {
      null
    }
  }
}
