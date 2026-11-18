package com.touchpad.windows

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import com.touchpad.protocol.MouseButton
import com.touchpad.protocol.TouchpadPacket
import java.awt.GraphicsEnvironment
import java.awt.MouseInfo
import java.awt.Point
import java.awt.Rectangle
import java.awt.Robot
import java.awt.event.InputEvent
import kotlin.math.abs

/**
 * Native Windows user32.dll mouse input interface for relative hardware-style deltas
 * required by 3D games (e.g. Minecraft) that lock the cursor and read raw relative motion.
 */
internal interface Win32User32 : Library {
    fun mouse_event(dwFlags: Int, dx: Int, dy: Int, dwData: Int, dwExtraInfo: Pointer?)

    companion object {
        const val MOUSEEVENTF_MOVE = 0x0001
        const val MOUSEEVENTF_LEFTDOWN = 0x0002
        const val MOUSEEVENTF_LEFTUP = 0x0004
        const val MOUSEEVENTF_RIGHTDOWN = 0x0008
        const val MOUSEEVENTF_RIGHTUP = 0x0010
        const val MOUSEEVENTF_MIDDLEDOWN = 0x0020
        const val MOUSEEVENTF_MIDDLEUP = 0x0040
        const val MOUSEEVENTF_WHEEL = 0x0800
        const val WHEEL_DELTA = 120
    }
}

/**
 * Abstraction over cursor and mouse input emission so control math can be verified
 * deterministically in headless unit tests without mutating the host cursor.
 */
interface ActionSink {
    val supportsRelativeMove: Boolean
        get() = false

    fun getCurrentPointerPosition(): Point?
    fun getMonitorBounds(): List<Rectangle>
    fun moveCursor(x: Int, y: Int)
    fun moveCursorRelative(dx: Int, dy: Int) {}
    fun mousePress(buttonMask: Int)
    fun mouseRelease(buttonMask: Int)
    fun mouseWheel(wheelTicks: Int)
}

/**
 * In-memory [ActionSink] for `dryRun` mode and automated tests.
 */
class DryRunActionSink(
    initialBounds: List<Rectangle> = listOf(Rectangle(0, 0, 1920, 1080)),
    initialPosition: Point = Point(960, 540),
    override val supportsRelativeMove: Boolean = false
) : ActionSink {
    private val bounds: List<Rectangle> = initialBounds.map { Rectangle(it) }

    @Volatile
    var cursorPosition: Point = Point(initialPosition)
        private set

    val relativeMoves: MutableList<Point> = mutableListOf()
    val pressedMasks: MutableSet<Int> = mutableSetOf()
    var pressCount: Int = 0
        private set
    var releaseCount: Int = 0
        private set
    var totalWheelTicks: Int = 0
        private set
    val wheelHistory: MutableList<Int> = mutableListOf()

    override fun getCurrentPointerPosition(): Point = Point(cursorPosition)

    override fun getMonitorBounds(): List<Rectangle> = bounds

    override fun moveCursor(x: Int, y: Int) {
        cursorPosition = Point(x, y)
    }

    override fun moveCursorRelative(dx: Int, dy: Int) {
        relativeMoves.add(Point(dx, dy))
        cursorPosition = Point(cursorPosition.x + dx, cursorPosition.y + dy)
    }

    override fun mousePress(buttonMask: Int) {
        pressedMasks.add(buttonMask)
        pressCount++
    }

    override fun mouseRelease(buttonMask: Int) {
        pressedMasks.remove(buttonMask)
        releaseCount++
    }

    override fun mouseWheel(wheelTicks: Int) {
        totalWheelTicks += wheelTicks
        wheelHistory.add(wheelTicks)
    }
}

/**
 * Production [ActionSink] backed by [java.awt.Robot] and AWT multi-monitor bounds.
 */
class RobotActionSink : ActionSink {
    private val robot: Robot? by lazy {
        try {
            if (GraphicsEnvironment.isHeadless()) {
                null
            } else {
                Robot().apply { autoDelay = 0 }
            }
        } catch (_: Exception) {
            null
        }
    }

    private val user32: Win32User32? by lazy {
        try {
            val isWindows = System.getProperty("os.name", "").lowercase().contains("win")
            if (!isWindows || GraphicsEnvironment.isHeadless()) {
                null
            } else {
                Native.load("user32", Win32User32::class.java)
            }
        } catch (_: Throwable) {
            null
        }
    }

    override val supportsRelativeMove: Boolean
        get() = user32 != null

    override fun getCurrentPointerPosition(): Point? {
        return try {
            MouseInfo.getPointerInfo()?.location
        } catch (_: Exception) {
            null
        }
    }

    override fun getMonitorBounds(): List<Rectangle> {
        return try {
            if (GraphicsEnvironment.isHeadless()) {
                listOf(DEFAULT_BOUNDS)
            } else {
                val devices = GraphicsEnvironment.getLocalGraphicsEnvironment().screenDevices
                val rects = devices.mapNotNull { device ->
                    device.defaultConfiguration?.bounds
                }.filter { it.width > 0 && it.height > 0 }
                rects.ifEmpty { listOf(DEFAULT_BOUNDS) }
            }
        } catch (_: Exception) {
            listOf(DEFAULT_BOUNDS)
        }
    }

    override fun moveCursor(x: Int, y: Int) {
        try {
            robot?.mouseMove(x, y)
        } catch (_: Exception) {
            // Avoid crashing the UDP thread if Windows switches to a secure desktop (UAC/lock screen).
        }
    }

    override fun moveCursorRelative(dx: Int, dy: Int) {
        val nativeUser32 = user32
        if (nativeUser32 != null) {
            try {
                nativeUser32.mouse_event(Win32User32.MOUSEEVENTF_MOVE, dx, dy, 0, null)
                return
            } catch (_: Throwable) {
            }
        }
        val current = getCurrentPointerPosition() ?: return
        moveCursor(current.x + dx, current.y + dy)
    }

    override fun mousePress(buttonMask: Int) {
        val nativeFlag = when (buttonMask) {
            InputEvent.BUTTON1_DOWN_MASK -> Win32User32.MOUSEEVENTF_LEFTDOWN
            InputEvent.BUTTON3_DOWN_MASK -> Win32User32.MOUSEEVENTF_RIGHTDOWN
            InputEvent.BUTTON2_DOWN_MASK -> Win32User32.MOUSEEVENTF_MIDDLEDOWN
            else -> 0
        }
        val nativeUser32 = user32
        if (nativeUser32 != null && nativeFlag != 0) {
            try {
                nativeUser32.mouse_event(nativeFlag, 0, 0, 0, null)
                return
            } catch (_: Throwable) {
            }
        }
        try {
            robot?.mousePress(buttonMask)
        } catch (_: Exception) {
        }
    }

    override fun mouseRelease(buttonMask: Int) {
        val nativeFlag = when (buttonMask) {
            InputEvent.BUTTON1_DOWN_MASK -> Win32User32.MOUSEEVENTF_LEFTUP
            InputEvent.BUTTON3_DOWN_MASK -> Win32User32.MOUSEEVENTF_RIGHTUP
            InputEvent.BUTTON2_DOWN_MASK -> Win32User32.MOUSEEVENTF_MIDDLEUP
            else -> 0
        }
        val nativeUser32 = user32
        if (nativeUser32 != null && nativeFlag != 0) {
            try {
                nativeUser32.mouse_event(nativeFlag, 0, 0, 0, null)
                return
            } catch (_: Throwable) {
            }
        }
        try {
            robot?.mouseRelease(buttonMask)
        } catch (_: Exception) {
        }
    }

    override fun mouseWheel(wheelTicks: Int) {
        if (wheelTicks == 0) return
        val nativeUser32 = user32
        if (nativeUser32 != null) {
            try {
                nativeUser32.mouse_event(
                    Win32User32.MOUSEEVENTF_WHEEL,
                    0,
                    0,
                    -wheelTicks * Win32User32.WHEEL_DELTA,
                    null
                )
                return
            } catch (_: Throwable) {
            }
        }
        try {
            robot?.mouseWheel(wheelTicks)
        } catch (_: Exception) {
        }
    }

    private companion object {
        val DEFAULT_BOUNDS = Rectangle(0, 0, 1920, 1080)
    }
}

/**
 * Translates incoming [TouchpadPacket] control commands into Windows pointer, click,
 * drag, and scroll actions with sub-pixel movement accumulation and multi-monitor clamping.
 */
class WindowsMouseController(
    val dryRun: Boolean = GraphicsEnvironment.isHeadless(),
    val actionSink: ActionSink = if (dryRun) DryRunActionSink() else RobotActionSink(),
    sensitivity: Float = DEFAULT_SENSITIVITY,
    scrollSensitivity: Float = DEFAULT_SCROLL_SENSITIVITY,
    private val scrollThrottleMs: Long = 0L,
    private val clockMs: () -> Long = System::currentTimeMillis
) {
    private val lock = Any()

    @Volatile
    var sensitivity: Float = if (sensitivity.isFinite()) {
        sensitivity.coerceIn(MIN_SENSITIVITY, MAX_SENSITIVITY)
    } else {
        DEFAULT_SENSITIVITY
    }
        set(value) {
            field = if (value.isFinite()) {
                value.coerceIn(MIN_SENSITIVITY, MAX_SENSITIVITY)
            } else {
                DEFAULT_SENSITIVITY
            }
        }

    private val safeScrollSensitivity: Float = if (scrollSensitivity.isFinite()) {
        scrollSensitivity.coerceIn(0.1f, 5.0f)
    } else {
        DEFAULT_SCROLL_SENSITIVITY
    }

    private var remainderX: Float = 0f
    private var remainderY: Float = 0f
    private var scrollAccumulator: Float = 0f
    private var lastScrollTimestampMs: Long = 0L
    private var lastKnownPosition: Point = actionSink.getCurrentPointerPosition() ?: Point(0, 0)
    private val pressedButtons: MutableSet<MouseButton> = mutableSetOf()

    @Volatile
    var totalClicks: Int = 0
        private set

    @Volatile
    var totalScrollTicks: Int = 0
        private set

    /**
     * Dispatches any pointer/mouse [TouchpadPacket] to the underlying [ActionSink].
     */
    fun handlePacket(packet: TouchpadPacket) {
        when (packet) {
            is TouchpadPacket.Move -> move(packet.dx, packet.dy)
            is TouchpadPacket.Click -> click(packet.button)
            is TouchpadPacket.ButtonState -> setButtonState(packet.button, packet.pressed)
            is TouchpadPacket.Scroll -> scroll(packet.deltaY)
            is TouchpadPacket.DiscoverRequest,
            is TouchpadPacket.DiscoverResponse,
            is TouchpadPacket.PairRequest,
            is TouchpadPacket.PairResponse,
            is TouchpadPacket.Ping,
            is TouchpadPacket.Pong -> Unit
        }
    }

    fun move(dx: Float, dy: Float) {
        if (!dx.isFinite() || !dy.isFinite()) return

        val clampedDx = dx.coerceIn(-MAX_MOVE_DELTA, MAX_MOVE_DELTA)
        val clampedDy = dy.coerceIn(-MAX_MOVE_DELTA, MAX_MOVE_DELTA)

        synchronized(lock) {
            if (clampedDx != 0f && clampedDx * remainderX < 0f) remainderX = 0f
            if (clampedDy != 0f && clampedDy * remainderY < 0f) remainderY = 0f

            val activeSensitivity = sensitivity
            val totalX = remainderX + clampedDx * activeSensitivity
            val totalY = remainderY + clampedDy * activeSensitivity

            if (!totalX.isFinite() || !totalY.isFinite()) {
                remainderX = 0f
                remainderY = 0f
                return
            }

            val stepX = totalX.toInt()
            val stepY = totalY.toInt()
            remainderX = totalX - stepX
            remainderY = totalY - stepY

            if (stepX == 0 && stepY == 0) return

            if (actionSink.supportsRelativeMove) {
                actionSink.moveCursorRelative(stepX, stepY)
                lastKnownPosition = Point(
                    (lastKnownPosition.x.toLong() + stepX.toLong())
                        .coerceIn(Int.MIN_VALUE.toLong(), Int.MAX_VALUE.toLong())
                        .toInt(),
                    (lastKnownPosition.y.toLong() + stepY.toLong())
                        .coerceIn(Int.MIN_VALUE.toLong(), Int.MAX_VALUE.toLong())
                        .toInt()
                )
                return
            }

            val current = actionSink.getCurrentPointerPosition() ?: lastKnownPosition
            val monitors = actionSink.getMonitorBounds()
            val targetX = current.x.toLong() + stepX.toLong()
            val targetY = current.y.toLong() + stepY.toLong()
            val clamped = clampToMonitors(targetX, targetY, monitors)

            if (clamped.x.toLong() != targetX) remainderX = 0f
            if (clamped.y.toLong() != targetY) remainderY = 0f

            lastKnownPosition = clamped
            if (clamped.x != current.x || clamped.y != current.y) {
                actionSink.moveCursor(clamped.x, clamped.y)
            }
        }
    }

    fun click(button: MouseButton) {
        synchronized(lock) {
            val mask = button.toAwtMask()
            if (pressedButtons.remove(button)) {
                actionSink.mouseRelease(mask)
            }
            actionSink.mousePress(mask)
            actionSink.mouseRelease(mask)
            totalClicks++
        }
    }

    fun setButtonState(button: MouseButton, pressed: Boolean) {
        synchronized(lock) {
            val mask = button.toAwtMask()
            if (pressed) {
                if (pressedButtons.add(button)) {
                    actionSink.mousePress(mask)
                }
            } else {
                if (pressedButtons.remove(button)) {
                    actionSink.mouseRelease(mask)
                }
            }
        }
    }

    fun scroll(deltaY: Float) {
        if (!deltaY.isFinite()) return

        val clampedDelta = deltaY.coerceIn(-MAX_SCROLL_DELTA, MAX_SCROLL_DELTA)

        synchronized(lock) {
            if (clampedDelta != 0f && clampedDelta * scrollAccumulator < 0f) {
                scrollAccumulator = 0f
            }

            scrollAccumulator += clampedDelta * safeScrollSensitivity
            if (!scrollAccumulator.isFinite()) {
                scrollAccumulator = 0f
                return
            }
            if (abs(scrollAccumulator) < 1.0f) return

            val rawTicks = scrollAccumulator.toInt()
            if (rawTicks == 0) return

            val now = clockMs()
            if (scrollThrottleMs > 0L && (now - lastScrollTimestampMs) < scrollThrottleMs) {
                return
            }

            val clampedTicks = rawTicks.coerceIn(-MAX_SCROLL_TICKS_PER_EVENT, MAX_SCROLL_TICKS_PER_EVENT)
            scrollAccumulator = if (clampedTicks != rawTicks) {
                0f
            } else {
                scrollAccumulator - rawTicks
            }
            lastScrollTimestampMs = now
            totalScrollTicks += clampedTicks
            actionSink.mouseWheel(clampedTicks)
        }
    }

    fun releaseAllButtons() {
        synchronized(lock) {
            if (pressedButtons.isEmpty()) return
            val active = pressedButtons.toList()
            pressedButtons.clear()
            for (button in active) {
                actionSink.mouseRelease(button.toAwtMask())
            }
        }
    }

    fun resetAccumulators() {
        synchronized(lock) {
            remainderX = 0f
            remainderY = 0f
            scrollAccumulator = 0f
        }
    }

    fun currentPosition(): Point {
        synchronized(lock) {
            return actionSink.getCurrentPointerPosition() ?: Point(lastKnownPosition)
        }
    }

    fun remainderSnapshot(): Pair<Float, Float> {
        synchronized(lock) {
            return remainderX to remainderY
        }
    }

    fun pressedButtonsSnapshot(): Set<MouseButton> {
        synchronized(lock) {
            return pressedButtons.toSet()
        }
    }

    private fun MouseButton.toAwtMask(): Int = when (this) {
        MouseButton.LEFT -> InputEvent.BUTTON1_DOWN_MASK
        MouseButton.RIGHT -> InputEvent.BUTTON3_DOWN_MASK
        MouseButton.MIDDLE -> InputEvent.BUTTON2_DOWN_MASK
    }

    companion object {
        const val MIN_SENSITIVITY = 0.1f
        const val MAX_SENSITIVITY = 10.0f
        const val DEFAULT_SENSITIVITY = 1.6f
        const val DEFAULT_SCROLL_SENSITIVITY = 1.0f
        private const val MAX_MOVE_DELTA = 10_000f
        private const val MAX_SCROLL_DELTA = 1_000f
        private const val MAX_SCROLL_TICKS_PER_EVENT = 12

        internal fun clampToMonitors(x: Long, y: Long, monitors: List<Rectangle>): Point {
            if (monitors.isEmpty()) {
                return Point(
                    x.coerceIn(0L, 1919L).toInt(),
                    y.coerceIn(0L, 1079L).toInt()
                )
            }

            for (rect in monitors) {
                val minX = rect.x.toLong()
                val maxX = minX + rect.width.toLong() - 1L
                val minY = rect.y.toLong()
                val maxY = minY + rect.height.toLong() - 1L
                if (x in minX..maxX && y in minY..maxY) {
                    return Point(x.toInt(), y.toInt())
                }
            }

            var bestX = 0
            var bestY = 0
            var bestDistanceSq = Long.MAX_VALUE

            for (rect in monitors) {
                val minX = rect.x.toLong()
                val maxX = minX + rect.width.toLong() - 1L
                val minY = rect.y.toLong()
                val maxY = minY + rect.height.toLong() - 1L

                val cx = x.coerceIn(minX, maxX)
                val cy = y.coerceIn(minY, maxY)
                val dx = x - cx
                val dy = y - cy
                val distSq = dx * dx + dy * dy
                if (distSq < bestDistanceSq) {
                    bestDistanceSq = distSq
                    bestX = cx.toInt()
                    bestY = cy.toInt()
                }
            }
            return Point(bestX, bestY)
        }
    }
}
