package org.retrorabbit.tv

import android.Manifest
import android.content.pm.PackageManager
import android.media.MediaMetadataRetriever
import android.os.Build
import android.os.Bundle
import android.view.KeyEvent
import android.view.WindowManager
import com.google.androidgamesdk.GameActivity
import org.json.JSONObject

/**
 * The window the Rust app (libretrorabbit.so) draws into. Besides hosting it, this
 * turns TV remote buttons into the keys the desktop app already knows, asks for
 * access to the TV's videos, and reads their metadata (ffprobe's job on desktop).
 */
class MainActivity : GameActivity() {
    @Volatile private var permissionGranted = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        val perm = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.READ_MEDIA_VIDEO
                   else Manifest.permission.READ_EXTERNAL_STORAGE
        if (checkSelfPermission(perm) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(perm), 1)
        }
    }

    override fun onRequestPermissionsResult(code: Int, perms: Array<out String>, results: IntArray) {
        super.onRequestPermissionsResult(code, perms, results)
        if (results.any { it == PackageManager.PERMISSION_GRANTED }) permissionGranted = true
    }

    /** Called from Rust about once a second. */
    fun takePermissionGranted(): Boolean {
        val g = permissionGranted
        permissionGranted = false
        return g
    }

    /** Called from Rust on its GL thread with the external texture to decode into. */
    fun makePlayer(texture: Int): TvPlayer = TvPlayer(this, texture)

    /** ffprobe-style JSON ({"format":{"duration":..,"tags":{..}}}) for one video file. */
    fun probe(path: String): String? {
        val r = MediaMetadataRetriever()
        return try {
            r.setDataSource(path)
            val ms = r.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: return null
            val tags = JSONObject()
            for ((key, id) in listOf(
                "title" to MediaMetadataRetriever.METADATA_KEY_TITLE,
                "genre" to MediaMetadataRetriever.METADATA_KEY_GENRE,
                "date" to MediaMetadataRetriever.METADATA_KEY_DATE,
                "year" to MediaMetadataRetriever.METADATA_KEY_YEAR,
                "artist" to MediaMetadataRetriever.METADATA_KEY_ARTIST,
                "album" to MediaMetadataRetriever.METADATA_KEY_ALBUM,
                "album_artist" to MediaMetadataRetriever.METADATA_KEY_ALBUMARTIST,
            )) {
                r.extractMetadata(id)?.takeIf { it.isNotBlank() }?.let { tags.put(key, it) }
            }
            JSONObject().put("format", JSONObject().put("duration", (ms / 1000.0).toString()).put("tags", tags)).toString()
        } catch (e: Exception) {
            null
        } finally {
            try { r.release() } catch (_: Exception) {}
        }
    }

    /**
     * Remote buttons → the desktop's keys: Channel +/- → Page Up/Down, Guide/Menu → G,
     * Info → I, Last channel → Backspace, Red/Green/Yellow/Blue → Like/Fav/Only-favs/Channels,
     * Back → Esc (the app decides: close a window, hide the toolbar, or recall), OK → Enter.
     */
    private fun remap(code: Int): Int? = when (code) {
        KeyEvent.KEYCODE_CHANNEL_UP -> KeyEvent.KEYCODE_PAGE_UP
        KeyEvent.KEYCODE_CHANNEL_DOWN -> KeyEvent.KEYCODE_PAGE_DOWN
        KeyEvent.KEYCODE_GUIDE, KeyEvent.KEYCODE_MENU, KeyEvent.KEYCODE_TV_CONTENTS_MENU -> KeyEvent.KEYCODE_G
        KeyEvent.KEYCODE_INFO -> KeyEvent.KEYCODE_I
        KeyEvent.KEYCODE_LAST_CHANNEL -> KeyEvent.KEYCODE_DEL
        KeyEvent.KEYCODE_PROG_RED -> KeyEvent.KEYCODE_L
        KeyEvent.KEYCODE_PROG_GREEN -> KeyEvent.KEYCODE_S
        KeyEvent.KEYCODE_PROG_YELLOW -> KeyEvent.KEYCODE_O
        KeyEvent.KEYCODE_PROG_BLUE -> KeyEvent.KEYCODE_C
        KeyEvent.KEYCODE_BACK -> KeyEvent.KEYCODE_ESCAPE
        KeyEvent.KEYCODE_DPAD_CENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> KeyEvent.KEYCODE_ENTER
        else -> null
    }

    override fun dispatchKeyEvent(e: KeyEvent): Boolean {
        val to = remap(e.keyCode) ?: return super.dispatchKeyEvent(e)
        return super.dispatchKeyEvent(
            KeyEvent(e.downTime, e.eventTime, e.action, to, e.repeatCount, 0, e.deviceId, e.scanCode, e.flags, e.source)
        )
    }
}
