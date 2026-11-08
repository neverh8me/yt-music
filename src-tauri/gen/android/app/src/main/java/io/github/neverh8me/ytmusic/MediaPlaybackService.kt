package io.github.neverh8me.ytmusic

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.os.Build
import android.os.IBinder
import android.support.v4.media.MediaMetadataCompat
import android.support.v4.media.session.MediaSessionCompat
import android.support.v4.media.session.PlaybackStateCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.media.session.MediaButtonReceiver

/**
 * System media integration: lock-screen / notification-shade player, headset & Bluetooth
 * buttons, Android Auto / Wear controls. Audio itself still plays in the WebView; this
 * service mirrors its state into a MediaSession and keeps the process in the foreground
 * while music plays so Android doesn't stop it in the background.
 */
class MediaPlaybackService : Service() {

    data class NowPlaying(
        val title: String,
        val artist: String,
        val album: String,
        val art: Bitmap?,
        val durationMs: Long,
        val positionMs: Long,
        val playing: Boolean,
    )

    companion object {
        private const val CHANNEL_ID = "playback"
        private const val NOTIFICATION_ID = 1

        @Volatile var instance: MediaPlaybackService? = null
        @Volatile private var pending: NowPlaying? = null

        /** Receives actions from the system (lock screen, headset...) → forwarded to the UI. */
        @Volatile var onAction: ((String, Long) -> Unit)? = null

        fun update(context: Context, state: NowPlaying) {
            val svc = instance
            if (svc != null) {
                svc.apply(state)
            } else {
                pending = state
                ContextCompat.startForegroundService(context, Intent(context, MediaPlaybackService::class.java))
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, MediaPlaybackService::class.java))
        }
    }

    private lateinit var session: MediaSessionCompat

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Playback", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Now playing controls"
                setShowBadge(false)
            }
        )
        session = MediaSessionCompat(this, "YTMusic").apply {
            setCallback(object : MediaSessionCompat.Callback() {
                override fun onPlay() { onAction?.invoke("play", 0) }
                override fun onPause() { onAction?.invoke("pause", 0) }
                override fun onSkipToNext() { onAction?.invoke("next", 0) }
                override fun onSkipToPrevious() { onAction?.invoke("prev", 0) }
                override fun onSeekTo(pos: Long) { onAction?.invoke("seek", pos) }
                override fun onStop() { onAction?.invoke("pause", 0) }
            })
            isActive = true
        }
        instance = this
        // Must enter the foreground quickly after startForegroundService().
        apply(pending ?: NowPlaying("YT Music", "", "", null, 0, 0, true))
        pending = null
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        MediaButtonReceiver.handleIntent(session, intent)
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        // App swiped away from recents: stop playback controls too.
        stopSelf()
    }

    override fun onDestroy() {
        instance = null
        session.isActive = false
        session.release()
        super.onDestroy()
    }

    fun apply(s: NowPlaying) {
        session.setMetadata(
            MediaMetadataCompat.Builder()
                .putString(MediaMetadataCompat.METADATA_KEY_TITLE, s.title)
                .putString(MediaMetadataCompat.METADATA_KEY_ARTIST, s.artist)
                .putString(MediaMetadataCompat.METADATA_KEY_ALBUM, s.album)
                .putLong(MediaMetadataCompat.METADATA_KEY_DURATION, s.durationMs)
                .apply { if (s.art != null) putBitmap(MediaMetadataCompat.METADATA_KEY_ALBUM_ART, s.art) }
                .build()
        )
        session.setPlaybackState(
            PlaybackStateCompat.Builder()
                .setActions(
                    PlaybackStateCompat.ACTION_PLAY or PlaybackStateCompat.ACTION_PAUSE or
                        PlaybackStateCompat.ACTION_PLAY_PAUSE or PlaybackStateCompat.ACTION_SKIP_TO_NEXT or
                        PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS or PlaybackStateCompat.ACTION_SEEK_TO or
                        PlaybackStateCompat.ACTION_STOP
                )
                .setState(
                    if (s.playing) PlaybackStateCompat.STATE_PLAYING else PlaybackStateCompat.STATE_PAUSED,
                    s.positionMs,
                    if (s.playing) 1f else 0f,
                )
                .build()
        )

        val open = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        fun action(icon: Int, title: String, act: Long) = NotificationCompat.Action(
            icon, title, MediaButtonReceiver.buildMediaButtonPendingIntent(this, act)
        )
        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(s.title)
            .setContentText(s.artist)
            .setLargeIcon(s.art)
            .setContentIntent(open)
            .setDeleteIntent(MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_STOP))
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setOngoing(s.playing)
            .addAction(action(android.R.drawable.ic_media_previous, "Previous", PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS))
            .addAction(
                if (s.playing) action(android.R.drawable.ic_media_pause, "Pause", PlaybackStateCompat.ACTION_PAUSE)
                else action(android.R.drawable.ic_media_play, "Play", PlaybackStateCompat.ACTION_PLAY)
            )
            .addAction(action(android.R.drawable.ic_media_next, "Next", PlaybackStateCompat.ACTION_SKIP_TO_NEXT))
            .setStyle(
                androidx.media.app.NotificationCompat.MediaStyle()
                    .setMediaSession(session.sessionToken)
                    .setShowActionsInCompactView(0, 1, 2)
            )
            .build()

        if (s.playing) {
            ServiceCompat.startForeground(
                this, NOTIFICATION_ID, notification,
                if (Build.VERSION.SDK_INT >= 29) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK else 0,
            )
        } else {
            // Paused: keep the controls but let Android reclaim the app if it needs to.
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_DETACH)
            getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification)
        }
    }
}
