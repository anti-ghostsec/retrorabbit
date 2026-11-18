package org.retrorabbit.tv

import android.content.Context
import android.graphics.SurfaceTexture
import android.media.audiofx.DynamicsProcessing
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.view.Surface
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.VideoSize
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.SeekParameters
import java.io.File

/**
 * ExoPlayer decoding into a SurfaceTexture on the Rust app's external GL texture.
 * Rust calls the methods below from its own threads; ExoPlayer itself only runs on
 * the main thread, so commands are posted there and state is read from volatiles.
 * Buffers stay in memory (no SimpleCache): nothing is written to disk.
 */
class TvPlayer(ctx: Context, texture: Int) : Player.Listener, SurfaceTexture.OnFrameAvailableListener {
    companion object {
        @JvmStatic external fun nativeWake()
    }

    private val main = Handler(Looper.getMainLooper())
    private val st = SurfaceTexture(texture)
    private val surface = Surface(st)
    private val matrix = FloatArray(16)
    private lateinit var exo: ExoPlayer
    private var dyn: DynamicsProcessing? = null

    @Volatile private var frame = false
    @Volatile private var ended = false
    @Volatile private var playing = false
    @Volatile private var err: String? = null
    @Volatile private var videoW = 0f
    @Volatile private var videoH = 0f
    // Events from a program we already switched away from are ignored.
    @Volatile private var wanted = 0
    private var current = 0
    private var volume = 1f
    private var muted = false
    private var even = true

    init {
        st.setOnFrameAvailableListener(this, main)
        main.post {
            exo = ExoPlayer.Builder(ctx).build()
            exo.setVideoSurface(surface)
            exo.addListener(this)
            // Start at the nearest keyframe: much quicker to tune in (mpv's --hr-seek=no).
            exo.setSeekParameters(SeekParameters.CLOSEST_SYNC)
            exo.setAudioAttributes(
                AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build(),
                false,
            )
            applyVolume()
            applyEven()
        }
    }

    override fun onFrameAvailable(t: SurfaceTexture) {
        frame = true
        nativeWake()
    }

    /** GL thread: latch the newest frame. out = 16 transform floats, video width, height. */
    fun update(out: FloatArray): Boolean {
        val fresh = frame
        if (fresh) {
            frame = false
            st.updateTexImage()
            st.getTransformMatrix(matrix)
        }
        System.arraycopy(matrix, 0, out, 0, 16)
        out[16] = videoW
        out[17] = videoH
        return fresh
    }

    /** Bits: 1 = ended on its own, 2 = picture on screen, 4 = failed (see error()). */
    fun state(): Int = (if (ended) 1 else 0) or (if (playing) 2 else 0) or (if (err != null) 4 else 0)

    fun error(): String? = err

    fun load(source: String, offsetMs: Long) {
        val gen = ++wanted
        ended = false
        playing = false
        err = null
        main.post {
            current = gen
            val uri = if (source.startsWith("/")) Uri.fromFile(File(source)) else Uri.parse(source)
            exo.setMediaItem(MediaItem.fromUri(uri), offsetMs)
            exo.prepare()
            exo.playWhenReady = true
        }
    }

    fun stop() {
        main.post {
            exo.stop()
            exo.clearMediaItems()
        }
    }

    // Block bodies: Rust looks these up as returning void.
    fun setVolume(v: Float) {
        main.post { volume = v; applyVolume() }
    }

    fun setMute(m: Boolean) {
        main.post { muted = m; applyVolume() }
    }

    fun setEvenVolume(on: Boolean) {
        main.post { even = on; applyEven() }
    }

    fun release() {
        main.post {
            dyn?.release()
            exo.release()
            surface.release()
            st.release()
        }
    }

    private fun applyVolume() {
        if (::exo.isInitialized) exo.volume = if (muted) 0f else volume
    }

    /** Even Volume: compress and lift quiet programs, limit loud ones (mpv's dynaudnorm on desktop). */
    private fun applyEven() {
        dyn?.release()
        dyn = null
        if (!even || !::exo.isInitialized) return
        val session = exo.audioSessionId
        if (session == C.AUDIO_SESSION_ID_UNSET) return
        try {
            val cfg = DynamicsProcessing.Config.Builder(
                DynamicsProcessing.VARIANT_FAVOR_FREQUENCY_RESOLUTION, 2,
                false, 0, true, 1, false, 0, true,
            ).build()
            dyn = DynamicsProcessing(0, session, cfg).apply {
                setMbcBandAllChannelsTo(0, DynamicsProcessing.MbcBand(
                    // enabled, cutoff Hz, attack ms, release ms, ratio, threshold dB,
                    // knee dB, noise gate dB, expander ratio, pre-gain dB, post-gain dB
                    true, 20000f, 20f, 800f, 3f, -32f, 8f, -90f, 1f, 0f, 9f,
                ))
                setLimiterAllChannelsTo(DynamicsProcessing.Limiter(true, true, 0, 1f, 80f, 10f, -3f, 0f))
                enabled = true
            }
        } catch (e: Exception) {
            dyn = null
        }
    }

    override fun onAudioSessionIdChanged(audioSessionId: Int) {
        applyEven()
    }

    override fun onVideoSizeChanged(size: VideoSize) {
        videoW = size.width * size.pixelWidthHeightRatio
        videoH = size.height.toFloat()
    }

    override fun onRenderedFirstFrame() {
        if (current != wanted) return
        playing = true
        nativeWake()
    }

    override fun onPlaybackStateChanged(state: Int) {
        if (current != wanted) return
        if (state == Player.STATE_ENDED) {
            ended = true
            nativeWake()
        }
    }

    override fun onPlayerError(error: PlaybackException) {
        if (current != wanted) return
        err = error.errorCodeName
        ended = true
        nativeWake()
    }
}
