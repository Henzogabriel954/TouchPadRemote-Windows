package com.touchpad.protocol

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.util.Locale

class TouchpadPacketTest {
    private lateinit var originalLocale: Locale

    @Before
    fun setUp() {
        originalLocale = Locale.getDefault()
    }

    @After
    fun tearDown() {
        Locale.setDefault(originalLocale)
    }

    @Test
    fun defaultPortIs45454() {
        assertEquals(45454, TouchpadPacket.DEFAULT_PORT)
    }

    @Test
    fun roundTripAllPacketTypesViaStringAndBytes() {
        val packets = listOf(
            TouchpadPacket.Move(12.5f, -3.75f),
            TouchpadPacket.Move(0f, 0f),
            TouchpadPacket.Move(0.1234f, -0.0005f),
            TouchpadPacket.Click(MouseButton.LEFT),
            TouchpadPacket.Click(MouseButton.RIGHT),
            TouchpadPacket.Click(MouseButton.MIDDLE),
            TouchpadPacket.ButtonState(MouseButton.LEFT, pressed = true),
            TouchpadPacket.ButtonState(MouseButton.RIGHT, pressed = false),
            TouchpadPacket.ButtonState(MouseButton.MIDDLE, pressed = true),
            TouchpadPacket.Scroll(4.25f),
            TouchpadPacket.Scroll(-12.5f),
            TouchpadPacket.DiscoverRequest(),
            TouchpadPacket.DiscoverRequest("Pixel 8 Pro (João)"),
            TouchpadPacket.DiscoverResponse(
                computerName = "Desktop-João",
                ipAddress = "192.168.1.42",
                port = 45454,
                method = ConnectionMethod.NETWORK_LAN
            ),
            TouchpadPacket.DiscoverResponse(
                computerName = "Workstation",
                ipAddress = "fe80::1",
                port = 1,
                method = ConnectionMethod.BLUETOOTH
            ),
            TouchpadPacket.DiscoverResponse(
                computerName = "Laptop",
                ipAddress = "10.0.0.5",
                port = 65535,
                method = ConnectionMethod.USB_ADB
            ),
            TouchpadPacket.Ping(0L),
            TouchpadPacket.Ping(Long.MAX_VALUE),
            TouchpadPacket.Ping(Long.MIN_VALUE),
            TouchpadPacket.Ping(1_730_000_000_123L),
            TouchpadPacket.Pong(1_730_000_000_456L, "Servidor Principal"),
            TouchpadPacket.Pong(Long.MAX_VALUE, "MaxServer"),
            TouchpadPacket.Pong(Long.MIN_VALUE, "MinServer"),
            TouchpadPacket.PairRequest(clientId = "device-uuid-01"),
            TouchpadPacket.PairRequest(clientId = "pixel-8-pro", clientName = "Pixel 8 Pro (João)"),
            TouchpadPacket.PairResponse(status = PairingStatus.PENDING, serverName = "Desktop-João"),
            TouchpadPacket.PairResponse(status = PairingStatus.APPROVED, serverName = "Workstation"),
            TouchpadPacket.PairResponse(status = PairingStatus.REJECTED, serverName = "Servidor Principal")
        )

        for (packet in packets) {
            val encoded = packet.encode()
            assertEquals("String round-trip failed for $packet ($encoded)", packet, TouchpadPacket.decode(encoded))

            val bytes = packet.toByteArray()
            assertEquals("ByteArray round-trip failed for $packet", packet, TouchpadPacket.fromByteArray(bytes))

            val udpBuffer = ByteArray(1024)
            bytes.copyInto(udpBuffer, destinationOffset = 0)
            assertEquals(
                "Datagram buffer slice round-trip failed for $packet",
                packet,
                TouchpadPacket.fromByteArray(udpBuffer, bytes.size)
            )
        }
    }

    @Test
    fun negativeZeroNormalizesCleanly() {
        val encoded = TouchpadPacket.Move(-0.0f, -0.0f).encode()
        assertFalse(encoded.contains("-0.0"))
        assertEquals(TouchpadPacket.Move(0f, 0f), TouchpadPacket.decode(encoded))
    }

    @Test
    fun fromByteArrayHandlesBufferBoundariesAndWhitespace() {
        val packet = TouchpadPacket.Pong(123456L, "Estação-São Paulo")
        val payload = "${packet.encode()}\r\n".toByteArray(Charsets.UTF_8)

        assertEquals(packet, TouchpadPacket.fromByteArray(payload, payload.size))
        assertNull(TouchpadPacket.fromByteArray(ByteArray(0)))
        assertNull(TouchpadPacket.fromByteArray(payload, length = 0))
        assertNull(TouchpadPacket.fromByteArray(payload, length = -5))
        assertNull(TouchpadPacket.fromByteArray(payload, length = payload.size + 1))
        assertNull(TouchpadPacket.fromByteArray(ByteArray(256)))
    }

    @Test
    fun connectionMethodFromIdAndAvailability() {
        assertTrue(ConnectionMethod.NETWORK_LAN.isAvailableNow)
        assertFalse(ConnectionMethod.BLUETOOTH.isAvailableNow)
        assertFalse(ConnectionMethod.USB_ADB.isAvailableNow)

        assertEquals(ConnectionMethod.NETWORK_LAN, ConnectionMethod.fromId("NETWORK_LAN"))
        assertEquals(ConnectionMethod.NETWORK_LAN, ConnectionMethod.fromId("network_lan"))
        assertEquals(ConnectionMethod.BLUETOOTH, ConnectionMethod.fromId("bluetooth"))
        assertEquals(ConnectionMethod.BLUETOOTH, ConnectionMethod.fromId("  bluetooth \r\n"))
        assertEquals(ConnectionMethod.USB_ADB, ConnectionMethod.fromId("Usb_Adb"))
        assertEquals(ConnectionMethod.USB_ADB, ConnectionMethod.fromId("  usb_adb "))
        assertEquals(ConnectionMethod.NETWORK_LAN, ConnectionMethod.fromId(""))
        assertEquals(ConnectionMethod.NETWORK_LAN, ConnectionMethod.fromId("UNKNOWN"))
    }

    @Test
    fun sanitizesPipesAndControlCharsInNamesAndIp() {
        val req = TouchpadPacket.DiscoverRequest("My|Phone\n\r\t\u0000Pro")
        val reqEncoded = req.encode()
        assertEquals(2, reqEncoded.split('|').size)
        val reqDecoded = TouchpadPacket.decode(reqEncoded) as? TouchpadPacket.DiscoverRequest
        assertNotNull(reqDecoded)
        assertFalse(reqDecoded!!.clientName.contains('|'))
        assertTrue(reqDecoded.clientName.none { it.isISOControl() })
        assertEquals("My Phone Pro", reqDecoded.clientName)

        val fallbackReq = TouchpadPacket.DiscoverRequest("|||\n\r\u0007")
        val fallbackReqDecoded = TouchpadPacket.decode(fallbackReq.encode()) as? TouchpadPacket.DiscoverRequest
        assertEquals(TouchpadPacket.DiscoverRequest("AndroidTouchpad"), fallbackReqDecoded)
        assertEquals(
            TouchpadPacket.DiscoverRequest("AndroidTouchpad"),
            TouchpadPacket.decode("DISCOVER_REQ|")
        )
        assertEquals(
            TouchpadPacket.DiscoverRequest("AndroidTouchpad"),
            TouchpadPacket.decode("DISCOVER_REQ|   ")
        )

        val resp = TouchpadPacket.DiscoverResponse(
            computerName = "Work|PC\n1\u0001",
            ipAddress = " 192.168.|1.10\r\n ",
            port = 45454
        )
        val respEncoded = resp.encode()
        assertEquals(5, respEncoded.split('|').size)
        val respDecoded = TouchpadPacket.decode(respEncoded) as? TouchpadPacket.DiscoverResponse
        assertNotNull(respDecoded)
        assertFalse(respDecoded!!.computerName.contains('|'))
        assertTrue(respDecoded.computerName.none { it.isISOControl() })
        assertEquals("192.168.1.10", respDecoded.ipAddress)

        val fallbackRespEncoded = TouchpadPacket.DiscoverResponse("|||", "10.0.0.1", 45454).encode()
        assertEquals("DISCOVER_RESP|Unknown|10.0.0.1|45454|NETWORK_LAN", fallbackRespEncoded)
        assertEquals(
            TouchpadPacket.DiscoverResponse("Unknown", "10.0.0.1", 45454),
            TouchpadPacket.decode(fallbackRespEncoded)
        )

        val pong = TouchpadPacket.Pong(777L, "Host|Srv\t\u001FMain")
        val pongEncoded = pong.encode()
        assertEquals(3, pongEncoded.split('|').size)
        val pongDecoded = TouchpadPacket.decode(pongEncoded) as? TouchpadPacket.Pong
        assertNotNull(pongDecoded)
        assertFalse(pongDecoded!!.serverName.contains('|'))
        assertTrue(pongDecoded.serverName.none { it.isISOControl() })
        assertEquals("Host Srv Main", pongDecoded.serverName)

        val fallbackPongEncoded = TouchpadPacket.Pong(1L, "|||").encode()
        assertEquals("PONG|1|Unknown", fallbackPongEncoded)
        assertEquals(TouchpadPacket.Pong(1L, "Unknown"), TouchpadPacket.decode(fallbackPongEncoded))

        val pairReq = TouchpadPacket.PairRequest(
            clientId = " id|123 \t\n\u0000abc ",
            clientName = "Tab|S9\r\n\u0007Ultra"
        )
        val pairReqEncoded = pairReq.encode()
        assertEquals(3, pairReqEncoded.split('|').size)
        val pairReqDecoded = TouchpadPacket.decode(pairReqEncoded) as? TouchpadPacket.PairRequest
        assertNotNull(pairReqDecoded)
        assertEquals("id123abc", pairReqDecoded!!.clientId)
        assertEquals("Tab S9 Ultra", pairReqDecoded.clientName)
        assertTrue(pairReqDecoded.clientId.none { it == '|' || it.isWhitespace() || it.isISOControl() })
        assertTrue(pairReqDecoded.clientName.none { it == '|' || it.isISOControl() })

        val fallbackPairReq = TouchpadPacket.PairRequest(
            clientId = " | \n\t\r ",
            clientName = "|||\u0000"
        )
        assertEquals(
            TouchpadPacket.PairRequest("android-default", "AndroidTouchpad"),
            TouchpadPacket.decode(fallbackPairReq.encode())
        )

        val pairResp = TouchpadPacket.PairResponse(
            status = PairingStatus.APPROVED,
            serverName = "Office|PC\n01\u0002"
        )
        val pairRespEncoded = pairResp.encode()
        assertEquals(3, pairRespEncoded.split('|').size)
        val pairRespDecoded = TouchpadPacket.decode(pairRespEncoded) as? TouchpadPacket.PairResponse
        assertEquals(
            TouchpadPacket.PairResponse(PairingStatus.APPROVED, "Office PC 01"),
            pairRespDecoded
        )

        val fallbackPairResp = TouchpadPacket.PairResponse(PairingStatus.REJECTED, "|||\r\n")
        assertEquals(
            TouchpadPacket.PairResponse(PairingStatus.REJECTED, "Unknown"),
            TouchpadPacket.decode(fallbackPairResp.encode())
        )
    }

    @Test
    fun localeIndependenceInPtBrGermanAndTurkishLocales() {
        val locales = listOf(
            Locale.forLanguageTag("pt-BR"),
            Locale.GERMANY,
            Locale.forLanguageTag("tr-TR")
        )

        for (locale in locales) {
            Locale.setDefault(locale)

            val move = TouchpadPacket.Move(12.34f, -56.78f)
            val moveEncoded = move.encode()
            assertFalse("Comma emitted under $locale: $moveEncoded", moveEncoded.contains(','))
            assertTrue(moveEncoded.contains('.'))
            assertEquals(move, TouchpadPacket.decode(moveEncoded))

            val scroll = TouchpadPacket.Scroll(-2.5f)
            val scrollEncoded = scroll.encode()
            assertFalse("Comma emitted under $locale: $scrollEncoded", scrollEncoded.contains(','))
            assertEquals(scroll, TouchpadPacket.decode(scrollEncoded))

            val click = TouchpadPacket.Click(MouseButton.MIDDLE)
            assertEquals(click, TouchpadPacket.decode("CLICK|middle"))
            assertEquals(
                TouchpadPacket.ButtonState(MouseButton.RIGHT, true),
                TouchpadPacket.decode("BUTTON_STATE|right|TRUE")
            )
            assertEquals(
                TouchpadPacket.DiscoverResponse("PC", "192.168.0.2", 45454, ConnectionMethod.USB_ADB),
                TouchpadPacket.decode("DISCOVER_RESP|PC|192.168.0.2|45454|usb_adb")
            )
            assertEquals(
                TouchpadPacket.PairResponse(PairingStatus.PENDING, "PC"),
                TouchpadPacket.decode("PAIR_RESP|pending|PC")
            )
            assertEquals(
                TouchpadPacket.PairResponse(PairingStatus.APPROVED, "PC"),
                TouchpadPacket.decode("PAIR_RESP|approved|PC")
            )
            assertEquals(
                TouchpadPacket.PairResponse(PairingStatus.REJECTED, "PC"),
                TouchpadPacket.decode("PAIR_RESP|rejected|PC")
            )

            assertNull(TouchpadPacket.decode("MOVE|12,34|-56,78"))
            assertNull(TouchpadPacket.decode("SCROLL|-2,5"))
        }
    }

    @Test
    fun malformedAndOutOfRangePacketsReturnNullWithoutThrowing() {
        val malformed = listOf(
            "",
            "   ",
            "\r\n\t",
            "UNKNOWN",
            "UNKNOWN|1|2",
            "MOVE",
            "MOVE|1.0",
            "MOVE|1.0|2.0|3.0",
            "MOVE|abc|1.0",
            "MOVE|1.0|xyz",
            "MOVE||1.0",
            "MOVE|1.0\u0000|2.0",
            "MOVE|NaN|1.0",
            "MOVE|1.0|Infinity",
            "MOVE|-Infinity|0.0",
            "SCROLL",
            "SCROLL|1.0|2.0",
            "SCROLL|NaN",
            "SCROLL|Infinity",
            "SCROLL|not_a_float",
            "CLICK",
            "CLICK|",
            "CLICK|SIDE",
            "CLICK|LEFT|EXTRA",
            "BUTTON_STATE",
            "BUTTON_STATE|LEFT",
            "BUTTON_STATE|LEFT|yes",
            "BUTTON_STATE|LEFT|1",
            "BUTTON_STATE|LEFT|",
            "BUTTON_STATE|UNKNOWN|true",
            "BUTTON_STATE|LEFT|true|extra",
            "DISCOVER_REQ",
            "DISCOVER_REQ|Phone|Extra",
            "DISCOVER_RESP",
            "DISCOVER_RESP|PC|192.168.1.1|45454",
            "DISCOVER_RESP||192.168.1.1|45454|NETWORK_LAN",
            "DISCOVER_RESP|PC||45454|NETWORK_LAN",
            "DISCOVER_RESP|PC|192.168.1.1|0|NETWORK_LAN",
            "DISCOVER_RESP|PC|192.168.1.1|-1|NETWORK_LAN",
            "DISCOVER_RESP|PC|192.168.1.1|65536|NETWORK_LAN",
            "DISCOVER_RESP|PC|192.168.1.1|abc|NETWORK_LAN",
            "DISCOVER_RESP|PC|192.168.1.1|45454|",
            "DISCOVER_RESP|PC|192.168.1.1|45454|NETWORK_LAN|EXTRA",
            "PING",
            "PING|",
            "PING|abc",
            "PING|9999999999999999999999",
            "PING|123|456",
            "PONG",
            "PONG|123",
            "PONG|abc|Server",
            "PONG|9999999999999999999999|Server",
            "PONG|123|",
            "PONG|123|   ",
            "PONG|123|Server|Extra",
            "PAIR_REQ",
            "PAIR_REQ|client-1",
            "PAIR_REQ||Phone",
            "PAIR_REQ|   |Phone",
            "PAIR_REQ|client-1|",
            "PAIR_REQ|client-1|   ",
            "PAIR_REQ|client-1|Phone|Extra",
            "PAIR_RESP",
            "PAIR_RESP|PENDING",
            "PAIR_RESP|PENDING|",
            "PAIR_RESP|PENDING|   ",
            "PAIR_RESP|UNKNOWN|Server",
            "PAIR_RESP||Server",
            "PAIR_RESP|PENDING\u0000|Server",
            "PAIR_RESP|APPROVED|Server|Extra"
        )

        for (raw in malformed) {
            assertNull("Expected null for malformed input: '$raw'", TouchpadPacket.decode(raw))
        }

        // Encoding invalid runtime values does not throw and decodes to null
        assertNull(TouchpadPacket.decode(TouchpadPacket.Move(Float.NaN, 1f).encode()))
        assertNull(TouchpadPacket.decode(TouchpadPacket.Move(1f, Float.POSITIVE_INFINITY).encode()))
        assertNull(TouchpadPacket.decode(TouchpadPacket.Scroll(Float.NEGATIVE_INFINITY).encode()))
        assertNull(
            TouchpadPacket.decode(
                TouchpadPacket.DiscoverResponse("PC", "192.168.1.1", 0).encode()
            )
        )
        assertNull(
            TouchpadPacket.decode(
                TouchpadPacket.DiscoverResponse("PC", "192.168.1.1", 70000).encode()
            )
        )
    }

    @Test
    fun unknownConnectionMethodFallsBackToNetworkLanInDiscoverResponse() {
        val decoded = TouchpadPacket.decode("DISCOVER_RESP|MyPC|192.168.1.20|45454|FUTURE_LINK")
        assertEquals(
            TouchpadPacket.DiscoverResponse("MyPC", "192.168.1.20", 45454, ConnectionMethod.NETWORK_LAN),
            decoded
        )
    }
}
