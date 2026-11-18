package com.touchpad.protocol

import java.util.Locale

enum class MouseButton {
    LEFT,
    RIGHT,
    MIDDLE
}

enum class PairingStatus {
    PENDING,
    APPROVED,
    REJECTED
}

sealed class TouchpadPacket {
    data class Move(val dx: Float, val dy: Float) : TouchpadPacket()
    data class Click(val button: MouseButton) : TouchpadPacket()
    data class ButtonState(val button: MouseButton, val pressed: Boolean) : TouchpadPacket()
    data class Scroll(val deltaY: Float) : TouchpadPacket()
    data class DiscoverRequest(val clientName: String = "AndroidTouchpad") : TouchpadPacket()
    data class DiscoverResponse(
        val computerName: String,
        val ipAddress: String,
        val port: Int,
        val method: ConnectionMethod = ConnectionMethod.NETWORK_LAN
    ) : TouchpadPacket()

    data class Ping(val timestamp: Long = System.currentTimeMillis()) : TouchpadPacket()
    data class Pong(val timestamp: Long, val serverName: String) : TouchpadPacket()
    data class PairRequest(
        val clientId: String,
        val clientName: String = "AndroidTouchpad"
    ) : TouchpadPacket()

    data class PairResponse(
        val status: PairingStatus,
        val serverName: String
    ) : TouchpadPacket()

    fun encode(): String = TouchpadCodec.encode(this)

    fun toByteArray(): ByteArray = encode().toByteArray(Charsets.UTF_8)

    companion object {
        const val DEFAULT_PORT: Int = 45454

        fun decode(raw: String): TouchpadPacket? = TouchpadCodec.decode(raw)

        fun fromByteArray(bytes: ByteArray, length: Int = bytes.size): TouchpadPacket? {
            if (length <= 0 || length > bytes.size) return null
            return decode(String(bytes, 0, length, Charsets.UTF_8).trim())
        }
    }
}

internal object TouchpadCodec {
    private const val DELIMITER_CHAR = '|'
    private const val TYPE_MOVE = "MOVE"
    private const val TYPE_CLICK = "CLICK"
    private const val TYPE_BUTTON_STATE = "BUTTON_STATE"
    private const val TYPE_SCROLL = "SCROLL"
    private const val TYPE_DISCOVER_REQ = "DISCOVER_REQ"
    private const val TYPE_DISCOVER_RESP = "DISCOVER_RESP"
    private const val TYPE_PING = "PING"
    private const val TYPE_PONG = "PONG"
    private const val TYPE_PAIR_REQ = "PAIR_REQ"
    private const val TYPE_PAIR_RESP = "PAIR_RESP"

    private const val DEFAULT_CLIENT_ID = "android-default"
    private const val DEFAULT_CLIENT_NAME = "AndroidTouchpad"
    private const val DEFAULT_SERVER_NAME = "Unknown"
    private val PORT_RANGE = 1..65535

    fun encode(packet: TouchpadPacket): String = when (packet) {
        is TouchpadPacket.Move ->
            "$TYPE_MOVE$DELIMITER_CHAR${formatFloat(packet.dx)}$DELIMITER_CHAR${formatFloat(packet.dy)}"

        is TouchpadPacket.Click ->
            "$TYPE_CLICK$DELIMITER_CHAR${packet.button.name}"

        is TouchpadPacket.ButtonState ->
            "$TYPE_BUTTON_STATE$DELIMITER_CHAR${packet.button.name}$DELIMITER_CHAR${packet.pressed}"

        is TouchpadPacket.Scroll ->
            "$TYPE_SCROLL$DELIMITER_CHAR${formatFloat(packet.deltaY)}"

        is TouchpadPacket.DiscoverRequest -> {
            val safeName = sanitizeText(packet.clientName).ifEmpty { DEFAULT_CLIENT_NAME }
            "$TYPE_DISCOVER_REQ$DELIMITER_CHAR$safeName"
        }

        is TouchpadPacket.DiscoverResponse -> {
            val safeComputer = sanitizeText(packet.computerName).ifEmpty { DEFAULT_SERVER_NAME }
            val safeIp = sanitizeCompactToken(packet.ipAddress)
            "$TYPE_DISCOVER_RESP$DELIMITER_CHAR$safeComputer$DELIMITER_CHAR$safeIp$DELIMITER_CHAR${packet.port}$DELIMITER_CHAR${packet.method.id}"
        }

        is TouchpadPacket.Ping ->
            "$TYPE_PING$DELIMITER_CHAR${packet.timestamp}"

        is TouchpadPacket.Pong -> {
            val safeServer = sanitizeText(packet.serverName).ifEmpty { DEFAULT_SERVER_NAME }
            "$TYPE_PONG$DELIMITER_CHAR${packet.timestamp}$DELIMITER_CHAR$safeServer"
        }

        is TouchpadPacket.PairRequest -> {
            val safeId = sanitizeCompactToken(packet.clientId).ifEmpty { DEFAULT_CLIENT_ID }
            val safeName = sanitizeText(packet.clientName).ifEmpty { DEFAULT_CLIENT_NAME }
            "$TYPE_PAIR_REQ$DELIMITER_CHAR$safeId$DELIMITER_CHAR$safeName"
        }

        is TouchpadPacket.PairResponse -> {
            val safeServer = sanitizeText(packet.serverName).ifEmpty { DEFAULT_SERVER_NAME }
            "$TYPE_PAIR_RESP$DELIMITER_CHAR${packet.status.name}$DELIMITER_CHAR$safeServer"
        }
    }

    fun decode(raw: String): TouchpadPacket? {
        val trimmed = raw.trim()
        if (trimmed.isEmpty()) return null

        val parts = trimmed.split(DELIMITER_CHAR)
        return when (parts[0]) {
            TYPE_MOVE -> decodeMove(parts)
            TYPE_CLICK -> decodeClick(parts)
            TYPE_BUTTON_STATE -> decodeButtonState(parts)
            TYPE_SCROLL -> decodeScroll(parts)
            TYPE_DISCOVER_REQ -> decodeDiscoverRequest(parts)
            TYPE_DISCOVER_RESP -> decodeDiscoverResponse(parts)
            TYPE_PING -> decodePing(parts)
            TYPE_PONG -> decodePong(parts)
            TYPE_PAIR_REQ -> decodePairRequest(parts)
            TYPE_PAIR_RESP -> decodePairResponse(parts)
            else -> null
        }
    }

    private fun decodeMove(parts: List<String>): TouchpadPacket.Move? {
        if (parts.size != 3) return null
        val dx = parseFiniteFloat(parts[1]) ?: return null
        val dy = parseFiniteFloat(parts[2]) ?: return null
        return TouchpadPacket.Move(dx, dy)
    }

    private fun decodeClick(parts: List<String>): TouchpadPacket.Click? {
        if (parts.size != 2) return null
        val button = parseMouseButton(parts[1]) ?: return null
        return TouchpadPacket.Click(button)
    }

    private fun decodeButtonState(parts: List<String>): TouchpadPacket.ButtonState? {
        if (parts.size != 3) return null
        val button = parseMouseButton(parts[1]) ?: return null
        val pressed = parseBoolean(parts[2]) ?: return null
        return TouchpadPacket.ButtonState(button, pressed)
    }

    private fun decodeScroll(parts: List<String>): TouchpadPacket.Scroll? {
        if (parts.size != 2) return null
        val deltaY = parseFiniteFloat(parts[1]) ?: return null
        return TouchpadPacket.Scroll(deltaY)
    }

    private fun decodeDiscoverRequest(parts: List<String>): TouchpadPacket.DiscoverRequest? {
        if (parts.size != 2) return null
        val clientName = sanitizeText(parts[1]).ifEmpty { DEFAULT_CLIENT_NAME }
        return TouchpadPacket.DiscoverRequest(clientName)
    }

    private fun decodeDiscoverResponse(parts: List<String>): TouchpadPacket.DiscoverResponse? {
        if (parts.size != 5) return null
        val computerName = sanitizeText(parts[1])
        if (computerName.isEmpty()) return null
        val ipAddress = sanitizeCompactToken(parts[2])
        if (ipAddress.isEmpty()) return null
        val port = parsePort(parts[3]) ?: return null
        val methodRaw = parts[4].trim()
        if (methodRaw.isEmpty() || methodRaw.any { it.isISOControl() }) return null
        val method = ConnectionMethod.fromId(methodRaw)
        return TouchpadPacket.DiscoverResponse(computerName, ipAddress, port, method)
    }

    private fun decodePing(parts: List<String>): TouchpadPacket.Ping? {
        if (parts.size != 2) return null
        val timestamp = parseLong(parts[1]) ?: return null
        return TouchpadPacket.Ping(timestamp)
    }

    private fun decodePong(parts: List<String>): TouchpadPacket.Pong? {
        if (parts.size != 3) return null
        val timestamp = parseLong(parts[1]) ?: return null
        val serverName = sanitizeText(parts[2])
        if (serverName.isEmpty()) return null
        return TouchpadPacket.Pong(timestamp, serverName)
    }

    private fun decodePairRequest(parts: List<String>): TouchpadPacket.PairRequest? {
        if (parts.size != 3) return null
        val clientId = sanitizeCompactToken(parts[1])
        if (clientId.isEmpty()) return null
        val clientName = sanitizeText(parts[2])
        if (clientName.isEmpty()) return null
        return TouchpadPacket.PairRequest(clientId, clientName)
    }

    private fun decodePairResponse(parts: List<String>): TouchpadPacket.PairResponse? {
        if (parts.size != 3) return null
        val status = parseEnum(parts[1], PairingStatus.entries) ?: return null
        val serverName = sanitizeText(parts[2])
        if (serverName.isEmpty()) return null
        return TouchpadPacket.PairResponse(status, serverName)
    }

    private fun formatFloat(value: Float): String {
        val normalized = if (value == 0f) 0f else value
        return String.format(Locale.US, "%s", normalized)
    }

    private fun parseFiniteFloat(token: String): Float? {
        if (token.any { it.isISOControl() }) return null
        val trimmed = token.trim()
        if (trimmed.isEmpty() || trimmed.contains(',')) return null
        val value = trimmed.toFloatOrNull() ?: return null
        if (!value.isFinite()) return null
        return if (value == 0f) 0f else value
    }

    private fun parsePort(token: String): Int? {
        if (token.any { it.isISOControl() }) return null
        val port = token.trim().toIntOrNull() ?: return null
        return port.takeIf { it in PORT_RANGE }
    }

    private fun parseLong(token: String): Long? {
        if (token.any { it.isISOControl() }) return null
        return token.trim().toLongOrNull()
    }

    private fun parseMouseButton(token: String): MouseButton? =
        parseEnum(token, MouseButton.entries)

    private fun <E : Enum<E>> parseEnum(token: String, entries: List<E>): E? {
        if (token.any { it.isISOControl() }) return null
        val normalized = token.trim().uppercase(Locale.US)
        return entries.firstOrNull { it.name == normalized }
    }

    private fun parseBoolean(token: String): Boolean? {
        if (token.any { it.isISOControl() }) return null
        return token.trim().lowercase(Locale.US).toBooleanStrictOrNull()
    }

    private fun sanitizeText(input: String): String =
        buildString(input.length) {
            var lastWasSpace = true
            for (ch in input) {
                when {
                    ch == DELIMITER_CHAR || ch == '\n' || ch == '\r' || ch == '\t' || ch == ' ' -> {
                        if (!lastWasSpace) {
                            append(' ')
                            lastWasSpace = true
                        }
                    }

                    ch.isISOControl() -> Unit

                    else -> {
                        append(ch)
                        lastWasSpace = false
                    }
                }
            }
        }.trimEnd()

    private fun sanitizeCompactToken(input: String): String =
        buildString(input.length) {
            for (ch in input) {
                if (ch != DELIMITER_CHAR && !ch.isISOControl() && !ch.isWhitespace()) {
                    append(ch)
                }
            }
        }
}
