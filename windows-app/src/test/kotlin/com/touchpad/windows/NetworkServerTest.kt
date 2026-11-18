package com.touchpad.windows

import com.touchpad.protocol.ConnectionMethod
import com.touchpad.protocol.MouseButton
import com.touchpad.protocol.PairingStatus
import com.touchpad.protocol.TouchpadPacket
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.awt.Point
import java.awt.Rectangle
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.util.Collections
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference

class NetworkServerTest {

    private val activeServers = mutableListOf<NetworkTouchpadServer>()

    @After
    fun tearDown() {
        activeServers.forEach { it.stop() }
        activeServers.clear()
    }

    private fun startTestServer(
        mouseController: WindowsMouseController = WindowsMouseController(dryRun = true),
        computerName: String = "Estacao-Teste",
        listener: ServerListener? = null,
        requirePairingApproval: Boolean = false
    ): NetworkTouchpadServer {
        return NetworkTouchpadServer(
            port = 0,
            mouseController = mouseController,
            computerName = computerName,
            listener = listener,
            requirePairingApproval = requirePairingApproval
        ).also {
            activeServers.add(it)
            it.start()
        }
    }

    @Test
    fun discoverRequestReturnsValidDiscoverResponse() {
        val server = startTestServer(computerName = "PC-Laboratorio")
        assertTrue(server.isRunning)
        assertTrue(server.actualPort > 0)

        DatagramSocket().use { clientSocket ->
            clientSocket.soTimeout = 2000
            val payload = TouchpadPacket.DiscoverRequest("Celular-Teste").toByteArray()
            val loopback = InetAddress.getByName("127.0.0.1")
            clientSocket.send(DatagramPacket(payload, payload.size, loopback, server.actualPort))

            val replyBuffer = ByteArray(2048)
            val replyPacket = DatagramPacket(replyBuffer, replyBuffer.size)
            clientSocket.receive(replyPacket)

            val decoded: TouchpadPacket? = TouchpadPacket.fromByteArray(replyPacket.data, replyPacket.length)
            assertTrue("Esperava DiscoverResponse, recebeu: $decoded", decoded is TouchpadPacket.DiscoverResponse)

            val response = decoded as TouchpadPacket.DiscoverResponse
            assertEquals("PC-Laboratorio", response.computerName)
            assertEquals(server.actualPort, response.port)
            assertEquals(ConnectionMethod.NETWORK_LAN, response.method)
            assertTrue(response.ipAddress.isNotBlank())
        }
    }

    @Test
    fun pingReturnsPongWithMatchingTimestampAndServerName() {
        val server = startTestServer(computerName = "PC-Escritorio")

        DatagramSocket().use { clientSocket ->
            clientSocket.soTimeout = 2000
            val pingTimestamp = 1_731_843_600_123L
            val payload = TouchpadPacket.Ping(pingTimestamp).toByteArray()
            val loopback = InetAddress.getByName("127.0.0.1")
            clientSocket.send(DatagramPacket(payload, payload.size, loopback, server.actualPort))

            val replyBuffer = ByteArray(2048)
            val replyPacket = DatagramPacket(replyBuffer, replyBuffer.size)
            clientSocket.receive(replyPacket)

            val decoded: TouchpadPacket? = TouchpadPacket.fromByteArray(replyPacket.data, replyPacket.length)
            assertTrue("Esperava Pong, recebeu: $decoded", decoded is TouchpadPacket.Pong)

            val pong = decoded as TouchpadPacket.Pong
            assertEquals(pingTimestamp, pong.timestamp)
            assertEquals("PC-Escritorio", pong.serverName)
        }
    }

    @Test
    fun moveAndClickUpdatePacketCountListenerAndMouseController() {
        val sink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540)
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = 2.0f
        )

        val latch = CountDownLatch(2)
        val receivedPackets = Collections.synchronizedList(mutableListOf<TouchpadPacket>())
        val receivedCounts = Collections.synchronizedList(mutableListOf<Long>())

        val listener = object : ServerListener {
            override fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {
                receivedPackets.add(packet)
                receivedCounts.add(totalPackets)
                latch.countDown()
            }
        }

        val server = startTestServer(mouseController = controller, listener = listener)

        DatagramSocket().use { clientSocket ->
            val loopback = InetAddress.getByName("127.0.0.1")

            val moveBytes = TouchpadPacket.Move(4.0f, -2.5f).toByteArray()
            clientSocket.send(DatagramPacket(moveBytes, moveBytes.size, loopback, server.actualPort))

            val clickBytes = TouchpadPacket.Click(MouseButton.LEFT).toByteArray()
            clientSocket.send(DatagramPacket(clickBytes, clickBytes.size, loopback, server.actualPort))
        }

        assertTrue("Timeout aguardando processamento de Move e Click", latch.await(2, TimeUnit.SECONDS))
        assertEquals(2L, server.packetCount)
        assertEquals("127.0.0.1", server.lastClientIp)
        assertEquals(listOf(1L, 2L), receivedCounts)
        assertTrue(receivedPackets[0] is TouchpadPacket.Move)
        assertTrue(receivedPackets[1] is TouchpadPacket.Click)

        assertEquals(Point(968, 535), sink.cursorPosition)
        assertEquals(1, sink.pressCount)
        assertEquals(1, sink.releaseCount)
        assertTrue(sink.pressedMasks.isEmpty())
    }

    @Test
    fun subPixelAccumulationMultiMonitorClampingAndScrollMath() {
        val sink = DryRunActionSink(
            initialBounds = listOf(
                Rectangle(0, 0, 1920, 1080),
                Rectangle(1920, 0, 1920, 1080)
            ),
            initialPosition = Point(1919, 540)
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = 1.0f
        )

        // Sub-pixel accumulation: 0.4 + 0.7 -> 1 pixel step with 0.1 remainder
        controller.handlePacket(TouchpadPacket.Move(0.4f, 0f))
        assertEquals(Point(1919, 540), sink.cursorPosition)

        controller.handlePacket(TouchpadPacket.Move(0.7f, 0f))
        assertEquals(Point(1920, 540), sink.cursorPosition)
        assertEquals(0.1f, controller.remainderSnapshot().first, 0.01f)

        // Clamping across second monitor outer edge
        controller.handlePacket(TouchpadPacket.Move(5000f, -2000f))
        assertEquals(Point(3839, 0), sink.cursorPosition)

        // Scroll accumulation across fractional gestures
        controller.handlePacket(TouchpadPacket.Scroll(0.4f))
        controller.handlePacket(TouchpadPacket.Scroll(0.4f))
        assertEquals(0, sink.totalWheelTicks)

        controller.handlePacket(TouchpadPacket.Scroll(0.4f))
        assertEquals(1, sink.totalWheelTicks)

        // Held drag button cleanly released on shutdown helper
        controller.handlePacket(TouchpadPacket.ButtonState(MouseButton.RIGHT, pressed = true))
        assertEquals(setOf(MouseButton.RIGHT), controller.pressedButtonsSnapshot())

        controller.releaseAllButtons()
        assertTrue(controller.pressedButtonsSnapshot().isEmpty())
        assertTrue(sink.pressedMasks.isEmpty())
    }

    @Test
    fun stopClosesSocketAndNotifiesListener() {
        val stateEvents = Collections.synchronizedList(mutableListOf<Pair<Boolean, Int>>())
        val listener = object : ServerListener {
            override fun onServerStateChanged(running: Boolean, port: Int) {
                stateEvents.add(running to port)
            }
        }

        val server = startTestServer(listener = listener)
        val boundPort = server.actualPort
        assertTrue(server.isRunning)

        server.stop()
        assertFalse(server.isRunning)
        assertEquals(listOf(true to boundPort, false to boundPort), stateEvents)
    }

    @Test
    fun localIpAddressesContainOnlyValidNonLoopbackIpv4() {
        val ipv4Pattern = Regex("""^\d{1,3}(\.\d{1,3}){3}$""")
        val ips = getLocalIpAddresses()
        for (ip in ips) {
            assertTrue("IP inválido retornado: $ip", ipv4Pattern.matches(ip))
            assertFalse("IP loopback não deveria aparecer: $ip", ip.startsWith("127."))
            assertFalse("IP link-local não deveria aparecer: $ip", ip.startsWith("169.254."))
        }
    }

    @Test
    fun pairingApprovalWorkflowBlocksUntilApproved() {
        val sink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540)
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = 1.0f
        )

        val pairingLatch = CountDownLatch(1)
        val moveLatch = CountDownLatch(1)
        val requestedClient = AtomicReference<PairingClientInfo?>(null)

        val listener = object : ServerListener {
            override fun onPairingRequested(client: PairingClientInfo) {
                requestedClient.set(client)
                pairingLatch.countDown()
            }

            override fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {
                if (packet is TouchpadPacket.Move) {
                    moveLatch.countDown()
                }
            }
        }

        val fakeClock = AtomicLong(10_000L)
        val server = startTestServer(
            mouseController = controller,
            computerName = "PC-Pareamento",
            listener = listener,
            requirePairingApproval = true
        )
        server.clockMs = { fakeClock.get() }

        val loopback = InetAddress.getByName("127.0.0.1")

        fun sendAndAwaitPingBarrier(socket: DatagramSocket) {
            val pingBytes = TouchpadPacket.Ping(42L).toByteArray()
            socket.send(DatagramPacket(pingBytes, pingBytes.size, loopback, server.actualPort))
            val buf = ByteArray(2048)
            val reply = DatagramPacket(buf, buf.size)
            socket.receive(reply)
            val decoded = TouchpadPacket.fromByteArray(reply.data, reply.length)
            assertTrue(decoded is TouchpadPacket.Pong)
        }

        fun sendPairRequest(socket: DatagramSocket): TouchpadPacket.PairResponse {
            val reqBytes = TouchpadPacket.PairRequest(
                clientId = "phone-uuid-123",
                clientName = "Pixel-Teste"
            ).toByteArray()
            socket.send(DatagramPacket(reqBytes, reqBytes.size, loopback, server.actualPort))
            val buf = ByteArray(2048)
            val reply = DatagramPacket(buf, buf.size)
            socket.receive(reply)
            val decoded: TouchpadPacket? = TouchpadPacket.fromByteArray(reply.data, reply.length)
            assertTrue("Esperava PairResponse, recebeu $decoded", decoded is TouchpadPacket.PairResponse)
            return decoded as TouchpadPacket.PairResponse
        }

        DatagramSocket().use { clientSocket ->
            clientSocket.soTimeout = 2000

            // 1. Move before pairing is ignored
            val unapprovedMove = TouchpadPacket.Move(10f, 5f).toByteArray()
            clientSocket.send(DatagramPacket(unapprovedMove, unapprovedMove.size, loopback, server.actualPort))
            sendAndAwaitPingBarrier(clientSocket)
            assertEquals(0L, server.packetCount)
            assertEquals(Point(960, 540), sink.cursorPosition)

            // 2. PairRequest returns PENDING and registers pending client
            val pendingResp = sendPairRequest(clientSocket)
            assertEquals(PairingStatus.PENDING, pendingResp.status)
            assertEquals("PC-Pareamento", pendingResp.serverName)
            assertTrue("Timeout aguardando onPairingRequested", pairingLatch.await(2, TimeUnit.SECONDS))
            assertEquals("phone-uuid-123", requestedClient.get()?.clientId)
            assertEquals("Pixel-Teste", requestedClient.get()?.clientName)
            assertEquals("127.0.0.1", requestedClient.get()?.ipAddress)
            assertEquals(1, server.getPendingPairings().size)
            assertTrue(server.getApprovedClients().isEmpty())

            // 3. Move while PENDING is still ignored
            clientSocket.send(DatagramPacket(unapprovedMove, unapprovedMove.size, loopback, server.actualPort))
            sendAndAwaitPingBarrier(clientSocket)
            assertEquals(0L, server.packetCount)
            assertEquals(Point(960, 540), sink.cursorPosition)

            // 4. Approving allows subsequent PairRequest -> APPROVED and Move execution
            server.approvePairing("127.0.0.1")
            assertTrue(server.getPendingPairings().isEmpty())
            assertEquals(1, server.getApprovedClients().size)

            val approvedResp = sendPairRequest(clientSocket)
            assertEquals(PairingStatus.APPROVED, approvedResp.status)
            assertEquals("PC-Pareamento", approvedResp.serverName)

            val approvedMove = TouchpadPacket.Move(12f, -4f).toByteArray()
            clientSocket.send(DatagramPacket(approvedMove, approvedMove.size, loopback, server.actualPort))
            assertTrue("Timeout aguardando Move aprovado", moveLatch.await(2, TimeUnit.SECONDS))
            assertEquals(1L, server.packetCount)
            assertEquals(Point(972, 536), sink.cursorPosition)

            // 5. Rejecting blocks Move and returns REJECTED during cooldown window
            server.rejectPairing("127.0.0.1")
            assertTrue(server.getApprovedClients().isEmpty())

            val rejectedResp = sendPairRequest(clientSocket)
            assertEquals(PairingStatus.REJECTED, rejectedResp.status)

            clientSocket.send(DatagramPacket(unapprovedMove, unapprovedMove.size, loopback, server.actualPort))
            sendAndAwaitPingBarrier(clientSocket)
            assertEquals(1L, server.packetCount)
            assertEquals(Point(972, 536), sink.cursorPosition)

            // 6. After 3500ms cooldown expires, phone can request pairing again
            fakeClock.addAndGet(4000L)
            val reRequestResp = sendPairRequest(clientSocket)
            assertEquals(PairingStatus.PENDING, reRequestResp.status)
            assertEquals(1, server.getPendingPairings().size)
        }
    }

    @Test
    fun relativeMoveModeEmitsPureDeltasWithoutAbsoluteClampingFor3dGames() {
        val sink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540),
            supportsRelativeMove = true
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = 1.0f
        )

        controller.handlePacket(TouchpadPacket.Move(500f, 800f))
        controller.handlePacket(TouchpadPacket.Move(-300f, -600f))
        controller.handlePacket(TouchpadPacket.Move(0.5f, 0.5f))
        controller.handlePacket(TouchpadPacket.Move(0.5f, 0.5f))

        assertEquals(
            listOf(Point(500, 800), Point(-300, -600), Point(1, 1)),
            sink.relativeMoves
        )
        assertEquals(Point(1161, 741), sink.cursorPosition)
    }

    @Test
    fun extremeFloatMaxValueMoveDoesNotPoisonAccumulators() {
        // 1. Absolute mode
        val absSink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540),
            supportsRelativeMove = false
        )
        val absController = WindowsMouseController(
            dryRun = true,
            actionSink = absSink,
            sensitivity = 1.0f
        )

        absController.handlePacket(TouchpadPacket.Move(Float.MAX_VALUE, -Float.MAX_VALUE))
        assertEquals(Point(1919, 0), absSink.cursorPosition)
        assertTrue(absController.remainderSnapshot().first.isFinite())
        assertTrue(absController.remainderSnapshot().second.isFinite())

        absController.handlePacket(TouchpadPacket.Move(-1f, 1f))
        assertEquals(Point(1918, 1), absSink.cursorPosition)

        // 2. Relative mode
        val relSink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540),
            supportsRelativeMove = true
        )
        val relController = WindowsMouseController(
            dryRun = true,
            actionSink = relSink,
            sensitivity = 1.0f
        )

        relController.handlePacket(TouchpadPacket.Move(Float.MAX_VALUE, -Float.MAX_VALUE))
        assertTrue(relController.remainderSnapshot().first.isFinite())
        assertTrue(relController.remainderSnapshot().second.isFinite())

        relController.handlePacket(TouchpadPacket.Move(1f, 1f))
        assertEquals(Point(1, 1), relSink.relativeMoves.last())
    }

    @Test
    fun nanSensitivityAndDirectionReversalResetRemainders() {
        val sink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540)
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = Float.NaN,
            scrollSensitivity = Float.NaN
        )

        assertEquals(WindowsMouseController.DEFAULT_SENSITIVITY, controller.sensitivity, 0.001f)

        // Direction reversal resets fractional cursor remainder
        controller.sensitivity = 1.0f
        controller.handlePacket(TouchpadPacket.Move(0.4f, 0.4f))
        assertEquals(0.4f, controller.remainderSnapshot().first, 0.01f)

        controller.handlePacket(TouchpadPacket.Move(-0.2f, -0.2f))
        assertEquals(-0.2f, controller.remainderSnapshot().first, 0.01f)
        assertEquals(-0.2f, controller.remainderSnapshot().second, 0.01f)

        // Direction reversal resets fractional scroll accumulator (sanitized to 1.0f scrollSensitivity)
        controller.handlePacket(TouchpadPacket.Scroll(0.4f))
        controller.handlePacket(TouchpadPacket.Scroll(-0.4f))
        controller.handlePacket(TouchpadPacket.Scroll(-0.4f))
        controller.handlePacket(TouchpadPacket.Scroll(-0.4f))
        assertEquals(-1, sink.totalWheelTicks)
    }

    @Test
    fun revokePairingReleasesHeldButtonsAndRejectsDuringCooldown() {
        val sink = DryRunActionSink(
            initialBounds = listOf(Rectangle(0, 0, 1920, 1080)),
            initialPosition = Point(960, 540)
        )
        val controller = WindowsMouseController(
            dryRun = true,
            actionSink = sink,
            sensitivity = 1.0f
        )
        val buttonLatch = CountDownLatch(1)
        val listener = object : ServerListener {
            override fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {
                if (packet is TouchpadPacket.ButtonState) {
                    buttonLatch.countDown()
                }
            }
        }

        val server = startTestServer(
            mouseController = controller,
            computerName = "PC-Revogacao",
            listener = listener,
            requirePairingApproval = true
        )
        val loopback = InetAddress.getByName("127.0.0.1")

        DatagramSocket().use { clientSocket ->
            clientSocket.soTimeout = 2000

            val reqBytes = TouchpadPacket.PairRequest("phone-drag", "Pixel-Drag").toByteArray()
            clientSocket.send(DatagramPacket(reqBytes, reqBytes.size, loopback, server.actualPort))
            val buf = ByteArray(2048)
            val reply = DatagramPacket(buf, buf.size)
            clientSocket.receive(reply)
            assertEquals(PairingStatus.PENDING, (TouchpadPacket.fromByteArray(reply.data, reply.length) as TouchpadPacket.PairResponse).status)

            server.approvePairing("127.0.0.1")

            val holdBytes = TouchpadPacket.ButtonState(MouseButton.LEFT, pressed = true).toByteArray()
            clientSocket.send(DatagramPacket(holdBytes, holdBytes.size, loopback, server.actualPort))
            assertTrue(buttonLatch.await(2, TimeUnit.SECONDS))
            assertEquals(setOf(MouseButton.LEFT), controller.pressedButtonsSnapshot())

            server.revokePairing("127.0.0.1")
            assertTrue(controller.pressedButtonsSnapshot().isEmpty())
            assertTrue(sink.pressedMasks.isEmpty())

            clientSocket.send(DatagramPacket(reqBytes, reqBytes.size, loopback, server.actualPort))
            val replyAfterRevoke = DatagramPacket(buf, buf.size)
            clientSocket.receive(replyAfterRevoke)
            val decoded = TouchpadPacket.fromByteArray(replyAfterRevoke.data, replyAfterRevoke.length) as TouchpadPacket.PairResponse
            assertEquals(PairingStatus.REJECTED, decoded.status)
        }
    }

    @Test
    fun ipMigrationByClientIdAndDifferentClientIdEviction() {
        val sink = DryRunActionSink()
        val controller = WindowsMouseController(dryRun = true, actionSink = sink)
        val buttonLatch = CountDownLatch(1)
        val server = startTestServer(
            mouseController = controller,
            computerName = "PC-Migracao",
            listener = object : ServerListener {
                override fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {
                    if (packet is TouchpadPacket.ButtonState) buttonLatch.countDown()
                }
            },
            requirePairingApproval = true
        )
        val loopback = InetAddress.getByName("127.0.0.1")

        fun sendPair(socket: DatagramSocket, clientId: String, clientName: String): PairingStatus {
            val req = TouchpadPacket.PairRequest(clientId, clientName).toByteArray()
            socket.send(DatagramPacket(req, req.size, loopback, server.actualPort))
            val buf = ByteArray(2048)
            val reply = DatagramPacket(buf, buf.size)
            socket.receive(reply)
            return (TouchpadPacket.fromByteArray(reply.data, reply.length) as TouchpadPacket.PairResponse).status
        }

        DatagramSocket().use { clientSocket ->
            clientSocket.soTimeout = 2000

            assertEquals(PairingStatus.PENDING, sendPair(clientSocket, "phone-alpha", "Celular A"))
            server.approvePairing("127.0.0.1")
            assertEquals(PairingStatus.APPROVED, sendPair(clientSocket, "phone-alpha", "Celular A"))

            // Migrate approval to another IP via clientId
            assertTrue(server.isClientApproved("10.0.0.99", "phone-alpha"))
            assertEquals(listOf("10.0.0.99"), server.getApprovedClients().map { it.ipAddress })
            assertFalse(server.isClientApproved("127.0.0.1"))

            // Phone A reconnects from 127.0.0.1 and migrates back cleanly
            assertEquals(PairingStatus.APPROVED, sendPair(clientSocket, "phone-alpha", "Celular A"))
            assertEquals(listOf("127.0.0.1"), server.getApprovedClients().map { it.ipAddress })

            // Hold a button, then a different clientId connects from the same IP -> evicts Phone A
            val hold = TouchpadPacket.ButtonState(MouseButton.LEFT, pressed = true).toByteArray()
            clientSocket.send(DatagramPacket(hold, hold.size, loopback, server.actualPort))
            assertTrue(buttonLatch.await(2, TimeUnit.SECONDS))
            assertEquals(setOf(MouseButton.LEFT), controller.pressedButtonsSnapshot())

            assertEquals(PairingStatus.PENDING, sendPair(clientSocket, "phone-beta", "Celular B"))
            assertTrue(server.getApprovedClients().isEmpty())
            assertTrue(controller.pressedButtonsSnapshot().isEmpty())
            assertEquals(listOf("phone-beta"), server.getPendingPairings().map { it.clientId })
        }
    }

    @Test
    fun resolveBestLocalIpForLoopbackReturnsLoopbackAddress() {
        val resolved = NetworkTouchpadServer.resolveBestLocalIpFor(InetAddress.getByName("127.0.0.1"))
        assertEquals("127.0.0.1", resolved)
    }
}
