package com.touchpad.windows

import com.touchpad.protocol.ConnectionMethod
import com.touchpad.protocol.PairingStatus
import com.touchpad.protocol.TouchpadPacket
import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InterfaceAddress
import java.net.NetworkInterface
import java.net.SocketException
import java.net.SocketTimeoutException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong

/**
 * Metadata representing a mobile device requesting or holding pairing authorization.
 */
data class PairingClientInfo(
    val clientId: String,
    val clientName: String,
    val ipAddress: String,
    val requestedAt: Long = System.currentTimeMillis()
)

/**
 * Listener notified of UDP server lifecycle transitions, pairing requests, and incoming remote packets.
 */
interface ServerListener {
    fun onServerStateChanged(running: Boolean, port: Int) {}
    fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {}
    fun onDiscoveryRequested(clientName: String, clientIp: String, respondedIp: String) {}
    fun onPingReceived(clientIp: String) {}
    fun onPairingRequested(client: PairingClientInfo) {}
    fun onPairingStateChanged(pending: List<PairingClientInfo>, approved: List<PairingClientInfo>) {}
    fun onServerError(message: String, cause: Throwable? = null) {}
}

/**
 * Non-blocking UDP server receiving [TouchpadPacket] datagrams from Android clients,
 * dispatching pointer input to [WindowsMouseController], and replying to discovery/ping/pairing requests.
 */
class NetworkTouchpadServer(
    val port: Int = TouchpadPacket.DEFAULT_PORT,
    val mouseController: WindowsMouseController = WindowsMouseController(),
    val computerName: String = resolveComputerName(),
    listener: ServerListener? = null,
    val requirePairingApproval: Boolean = false
) {
    private val lifecycleLock = Any()
    private val pairingLock = Any()
    private val _packetCount = AtomicLong(0L)

    private val approvedClients = ConcurrentHashMap<String, PairingClientInfo>()
    private val pendingClients = ConcurrentHashMap<String, PairingClientInfo>()
    private val rejectedTimestamps = ConcurrentHashMap<String, Long>()

    internal var clockMs: () -> Long = System::currentTimeMillis

    @Volatile
    var listener: ServerListener? = listener

    @Volatile
    var isRunning: Boolean = false
        private set

    @Volatile
    var actualPort: Int = if (port > 0) port else 0
        private set

    @Volatile
    var lastClientIp: String? = null
        private set

    val packetCount: Long
        get() = _packetCount.get()

    private var socket: DatagramSocket? = null
    private var workerThread: Thread? = null

    /**
     * Binds the UDP socket synchronously so [actualPort] is immediately available,
     * then starts a dedicated daemon thread to receive and process incoming packets.
     */
    @Throws(IOException::class)
    fun start() {
        val boundPort: Int
        synchronized(lifecycleLock) {
            if (isRunning) return

            val boundSocket = try {
                DatagramSocket(port).apply {
                    soTimeout = SOCKET_TIMEOUT_MS
                    try {
                        receiveBufferSize = RECEIVE_BUFFER_BYTES
                    } catch (_: SocketException) {
                        // Some virtual environments restrict SO_RCVBUF; safe to keep default.
                    }
                }
            } catch (ex: IOException) {
                notifyError("Não foi possível iniciar o servidor na porta $port: ${ex.message}", ex)
                throw ex
            }

            socket = boundSocket
            boundPort = boundSocket.localPort
            actualPort = boundPort
            isRunning = true

            val thread = Thread({ runReceiveLoop(boundSocket) }, "TouchpadUdpServer-$boundPort").apply {
                isDaemon = true
            }
            workerThread = thread
            thread.start()
        }

        notifyStateChanged(running = true, port = boundPort)
    }

    /**
     * Gracefully closes the UDP socket, waits briefly for the worker thread to exit,
     * and releases any held mouse buttons so drags never stay stuck.
     */
    fun stop() {
        val threadToJoin: Thread?
        val closedPort: Int
        val hadPairings: Boolean
        synchronized(lifecycleLock) {
            if (!isRunning && socket == null) return
            isRunning = false
            closedPort = actualPort
            threadToJoin = workerThread
            workerThread = null
            try {
                socket?.close()
            } catch (_: Exception) {
            } finally {
                socket = null
            }
            synchronized(pairingLock) {
                hadPairings = pendingClients.isNotEmpty() || approvedClients.isNotEmpty()
                pendingClients.clear()
                approvedClients.clear()
                rejectedTimestamps.clear()
                mouseController.releaseAllButtons()
                mouseController.resetAccumulators()
            }
        }

        if (threadToJoin != null && Thread.currentThread() !== threadToJoin) {
            try {
                threadToJoin.join(STOP_JOIN_TIMEOUT_MS)
            } catch (_: InterruptedException) {
                Thread.currentThread().interrupt()
            }
        }

        if (hadPairings) {
            notifyPairingStateChanged(emptyList(), emptyList())
        }
        notifyStateChanged(running = false, port = closedPort)
    }

    fun approvePairing(ipAddress: String) {
        val pendingSnap: List<PairingClientInfo>
        val approvedSnap: List<PairingClientInfo>
        synchronized(pairingLock) {
            val pending = pendingClients.remove(ipAddress)
            rejectedTimestamps.remove(ipAddress)

            val existing = approvedClients[ipAddress]
            val approvedInfo = pending ?: existing ?: PairingClientInfo(
                clientId = "",
                clientName = ipAddress,
                ipAddress = ipAddress,
                requestedAt = clockMs()
            )

            if (approvedInfo.clientId.isNotBlank()) {
                approvedClients.entries.removeIf { (key, info) ->
                    key != ipAddress && info.clientId == approvedInfo.clientId
                }
                pendingClients.entries.removeIf { (_, info) ->
                    info.clientId == approvedInfo.clientId
                }
            }
            approvedClients[ipAddress] = approvedInfo
            pendingSnap = getPendingPairingsLocked()
            approvedSnap = getApprovedClientsLocked()
        }
        notifyPairingStateChanged(pendingSnap, approvedSnap)
    }

    fun rejectPairing(ipAddress: String) {
        val pendingSnap: List<PairingClientInfo>
        val approvedSnap: List<PairingClientInfo>
        synchronized(pairingLock) {
            pendingClients.remove(ipAddress)
            approvedClients.remove(ipAddress)
            rejectedTimestamps[ipAddress] = clockMs()
            mouseController.releaseAllButtons()
            mouseController.resetAccumulators()
            pendingSnap = getPendingPairingsLocked()
            approvedSnap = getApprovedClientsLocked()
        }
        notifyPairingStateChanged(pendingSnap, approvedSnap)
    }

    fun revokePairing(ipAddress: String) {
        var pendingSnap: List<PairingClientInfo>? = null
        var approvedSnap: List<PairingClientInfo>? = null
        synchronized(pairingLock) {
            val removedApproved = approvedClients.remove(ipAddress)
            val removedPending = pendingClients.remove(ipAddress)
            if (removedApproved != null || removedPending != null) {
                rejectedTimestamps[ipAddress] = clockMs()
                mouseController.releaseAllButtons()
                mouseController.resetAccumulators()
                pendingSnap = getPendingPairingsLocked()
                approvedSnap = getApprovedClientsLocked()
            }
        }
        val pSnap = pendingSnap
        val aSnap = approvedSnap
        if (pSnap != null && aSnap != null) {
            notifyPairingStateChanged(pSnap, aSnap)
        }
    }

    fun revokeAllPairings() {
        val hadAny: Boolean
        synchronized(pairingLock) {
            hadAny = approvedClients.isNotEmpty() || pendingClients.isNotEmpty()
            approvedClients.clear()
            pendingClients.clear()
            rejectedTimestamps.clear()
            mouseController.releaseAllButtons()
            mouseController.resetAccumulators()
        }
        if (hadAny) {
            notifyPairingStateChanged(emptyList(), emptyList())
        }
    }

    fun getPendingPairings(): List<PairingClientInfo> = synchronized(pairingLock) {
        getPendingPairingsLocked()
    }

    fun getApprovedClients(): List<PairingClientInfo> = synchronized(pairingLock) {
        getApprovedClientsLocked()
    }

    private fun getPendingPairingsLocked(): List<PairingClientInfo> =
        pendingClients.values.sortedBy { it.requestedAt }

    private fun getApprovedClientsLocked(): List<PairingClientInfo> =
        approvedClients.values.sortedBy { it.requestedAt }

    private fun isClientApprovedLocked(ipAddress: String): Boolean =
        !requirePairingApproval || approvedClients.containsKey(ipAddress)

    fun isClientApproved(ipAddress: String, clientId: String? = null): Boolean {
        var pendingSnap: List<PairingClientInfo>? = null
        var approvedSnap: List<PairingClientInfo>? = null
        val approved: Boolean
        synchronized(pairingLock) {
            val (result, changed) = checkAndMigrateApprovedClientLocked(ipAddress, clientId)
            approved = result
            if (changed) {
                pendingSnap = getPendingPairingsLocked()
                approvedSnap = getApprovedClientsLocked()
            }
        }
        val pSnap = pendingSnap
        val aSnap = approvedSnap
        if (pSnap != null && aSnap != null) {
            notifyPairingStateChanged(pSnap, aSnap)
        }
        return approved
    }

    private fun checkAndMigrateApprovedClientLocked(
        ipAddress: String,
        clientId: String?
    ): Pair<Boolean, Boolean> {
        if (!requirePairingApproval) return true to false

        val existingAtIp = approvedClients[ipAddress]
        if (clientId.isNullOrBlank()) {
            return (existingAtIp != null) to false
        }

        if (existingAtIp != null && (existingAtIp.clientId.isBlank() || existingAtIp.clientId == clientId)) {
            var changed = false
            if (existingAtIp.clientId.isBlank()) {
                approvedClients[ipAddress] = existingAtIp.copy(clientId = clientId)
                changed = true
            }
            if (approvedClients.entries.removeIf { (key, info) ->
                    key != ipAddress && info.clientId == clientId
                }
            ) {
                changed = true
            }
            if (pendingClients.entries.removeIf { (key, info) ->
                    key == ipAddress || info.clientId == clientId
                }
            ) {
                changed = true
            }
            rejectedTimestamps.remove(ipAddress)
            return true to changed
        }

        val match = approvedClients.entries.firstOrNull { (_, info) ->
            info.clientId.isNotBlank() && info.clientId == clientId
        }
        if (match != null) {
            val updated = match.value.copy(ipAddress = ipAddress)
            approvedClients.entries.removeIf { (key, info) ->
                key != ipAddress && info.clientId == clientId
            }
            approvedClients[ipAddress] = updated
            pendingClients.entries.removeIf { (key, info) ->
                key == ipAddress || info.clientId == clientId
            }
            rejectedTimestamps.remove(ipAddress)
            return true to true
        }
        return false to false
    }

    private fun runReceiveLoop(activeSocket: DatagramSocket) {
        val buffer = ByteArray(MAX_PACKET_BYTES)
        val datagram = DatagramPacket(buffer, buffer.size)

        try {
            while (isRunning && !activeSocket.isClosed) {
                datagram.length = buffer.size
                try {
                    activeSocket.receive(datagram)
                } catch (_: SocketTimeoutException) {
                    continue
                } catch (ex: SocketException) {
                    if (!isRunning || activeSocket.isClosed) break
                    notifyError("Erro no socket UDP: ${ex.message}", ex)
                    continue
                } catch (ex: IOException) {
                    if (!isRunning || activeSocket.isClosed) break
                    notifyError("Falha de E/S ao receber pacote UDP: ${ex.message}", ex)
                    continue
                }

                handleDatagram(activeSocket, datagram)
            }
        } finally {
            mouseController.releaseAllButtons()
        }
    }

    private fun handleDatagram(activeSocket: DatagramSocket, datagram: DatagramPacket) {
        val now = clockMs()
        if (rejectedTimestamps.isNotEmpty()) {
            synchronized(pairingLock) {
                rejectedTimestamps.entries.removeIf { (_, timestamp) ->
                    now - timestamp > REJECT_COOLDOWN_MS
                }
            }
        }

        val packet: TouchpadPacket? = try {
            TouchpadPacket.fromByteArray(datagram.data, datagram.length)
        } catch (_: Exception) {
            null
        }
        if (packet == null) return

        val senderAddress = datagram.address ?: return
        val senderIp = senderAddress.hostAddress ?: return

        when (packet) {
            is TouchpadPacket.Move,
            is TouchpadPacket.Click,
            is TouchpadPacket.ButtonState,
            is TouchpadPacket.Scroll -> {
                val count: Long
                synchronized(pairingLock) {
                    if (!isRunning || !isClientApprovedLocked(senderIp)) return
                    mouseController.handlePacket(packet)
                    lastClientIp = senderIp
                    count = _packetCount.incrementAndGet()
                }
                try {
                    listener?.onPacketReceived(packet, senderIp, count)
                } catch (_: Exception) {
                }
            }

            is TouchpadPacket.DiscoverRequest -> {
                val localIp = resolveBestLocalIpFor(senderAddress)
                    ?: getLocalIpAddresses().firstOrNull()
                    ?: senderAddress.takeIf { it.isLoopbackAddress }?.hostAddress
                    ?: "127.0.0.1"
                val replyBytes = TouchpadPacket.DiscoverResponse(
                    computerName = computerName,
                    ipAddress = localIp,
                    port = actualPort,
                    method = ConnectionMethod.NETWORK_LAN
                ).toByteArray()
                sendReply(activeSocket, replyBytes, senderAddress, datagram.port)
                try {
                    listener?.onDiscoveryRequested(packet.clientName, senderIp, localIp)
                } catch (_: Exception) {
                }
            }

            is TouchpadPacket.Ping -> {
                val replyBytes = TouchpadPacket.Pong(
                    timestamp = packet.timestamp,
                    serverName = computerName
                ).toByteArray()
                sendReply(activeSocket, replyBytes, senderAddress, datagram.port)
                try {
                    listener?.onPingReceived(senderIp)
                } catch (_: Exception) {
                }
            }

            is TouchpadPacket.PairRequest -> {
                handlePairRequest(activeSocket, datagram, senderAddress, senderIp, packet, now)
            }

            is TouchpadPacket.DiscoverResponse,
            is TouchpadPacket.Pong,
            is TouchpadPacket.PairResponse -> Unit
        }
    }

    private fun handlePairRequest(
        activeSocket: DatagramSocket,
        datagram: DatagramPacket,
        senderAddress: InetAddress,
        senderIp: String,
        packet: TouchpadPacket.PairRequest,
        now: Long
    ) {
        var promptClient: PairingClientInfo? = null
        var pendingSnap: List<PairingClientInfo>? = null
        var approvedSnap: List<PairingClientInfo>? = null

        val responseStatus: PairingStatus = synchronized(pairingLock) {
            val previousApprovedAtIp = approvedClients[senderIp]
            val (approved, migrated) = checkAndMigrateApprovedClientLocked(senderIp, packet.clientId)
            if (approved) {
                if (requirePairingApproval) {
                    val existingAtIp = approvedClients[senderIp]
                    val updatedInfo = PairingClientInfo(
                        clientId = packet.clientId,
                        clientName = packet.clientName,
                        ipAddress = senderIp,
                        requestedAt = existingAtIp?.requestedAt ?: now
                    )
                    approvedClients[senderIp] = updatedInfo
                    pendingClients.remove(senderIp)
                    if (migrated ||
                        previousApprovedAtIp == null ||
                        previousApprovedAtIp.clientId != updatedInfo.clientId ||
                        previousApprovedAtIp.clientName != updatedInfo.clientName
                    ) {
                        pendingSnap = getPendingPairingsLocked()
                        approvedSnap = getApprovedClientsLocked()
                    }
                }
                PairingStatus.APPROVED
            } else {
                var evictedDifferentClient = false
                val staleAtIp = approvedClients[senderIp]
                if (staleAtIp != null &&
                    staleAtIp.clientId.isNotBlank() &&
                    staleAtIp.clientId != packet.clientId
                ) {
                    approvedClients.remove(senderIp)
                    mouseController.releaseAllButtons()
                    mouseController.resetAccumulators()
                    evictedDifferentClient = true
                }

                if (rejectedTimestamps.containsKey(senderIp)) {
                    if (evictedDifferentClient) {
                        pendingSnap = getPendingPairingsLocked()
                        approvedSnap = getApprovedClientsLocked()
                    }
                    PairingStatus.REJECTED
                } else {
                    val removedOtherPending = if (packet.clientId.isNotBlank()) {
                        pendingClients.entries.removeIf { (key, info) ->
                            key != senderIp && info.clientId == packet.clientId
                        }
                    } else {
                        false
                    }

                    val existingPending = pendingClients[senderIp]
                    val isNewOrChanged = existingPending == null ||
                        existingPending.clientId != packet.clientId ||
                        existingPending.clientName != packet.clientName
                    val isStaleReprompt = existingPending != null && (now - existingPending.requestedAt) >= REPROMPT_COOLDOWN_MS

                    val info = if (existingPending == null || isNewOrChanged || isStaleReprompt) {
                        PairingClientInfo(
                            clientId = packet.clientId,
                            clientName = packet.clientName,
                            ipAddress = senderIp,
                            requestedAt = now
                        ).also { pendingClients[senderIp] = it }
                    } else {
                        existingPending
                    }

                    if (isNewOrChanged || isStaleReprompt) {
                        promptClient = info
                    }
                    if (isNewOrChanged || evictedDifferentClient || removedOtherPending) {
                        pendingSnap = getPendingPairingsLocked()
                        approvedSnap = getApprovedClientsLocked()
                    }
                    PairingStatus.PENDING
                }
            }
        }

        val replyBytes = TouchpadPacket.PairResponse(
            status = responseStatus,
            serverName = computerName
        ).toByteArray()
        sendReply(activeSocket, replyBytes, senderAddress, datagram.port)

        promptClient?.let { notifyPairingRequested(it) }
        val pSnap = pendingSnap
        val aSnap = approvedSnap
        if (pSnap != null && aSnap != null) {
            notifyPairingStateChanged(pSnap, aSnap)
        }
    }

    private fun sendReply(
        activeSocket: DatagramSocket,
        payload: ByteArray,
        address: InetAddress,
        targetPort: Int
    ) {
        if (targetPort !in 1..65535 || activeSocket.isClosed) return
        try {
            val replyPacket = DatagramPacket(payload, payload.size, address, targetPort)
            activeSocket.send(replyPacket)
        } catch (_: Exception) {
            // Client socket may have closed or network changed; ignore transient send errors.
        }
    }

    private fun notifyPairingRequested(client: PairingClientInfo) {
        try {
            listener?.onPairingRequested(client)
        } catch (_: Exception) {
        }
    }

    private fun notifyPairingStateChanged(
        pending: List<PairingClientInfo>,
        approved: List<PairingClientInfo>
    ) {
        try {
            listener?.onPairingStateChanged(pending, approved)
        } catch (_: Exception) {
        }
    }

    private fun notifyStateChanged(running: Boolean, port: Int) {
        try {
            listener?.onServerStateChanged(running, port)
        } catch (_: Exception) {
        }
    }

    private fun notifyError(message: String, cause: Throwable?) {
        try {
            listener?.onServerError(message, cause)
        } catch (_: Exception) {
        }
    }

    companion object {
        private const val SOCKET_TIMEOUT_MS = 500
        private const val STOP_JOIN_TIMEOUT_MS = 1000L
        private const val MAX_PACKET_BYTES = 2048
        private const val RECEIVE_BUFFER_BYTES = 65536
        internal const val REJECT_COOLDOWN_MS = 3500L
        private const val REPROMPT_COOLDOWN_MS = 15000L

        private data class NetworkCandidate(
            val ip: String,
            val address: Inet4Address,
            val prefixLength: Int,
            val score: Int
        )

        /**
         * Returns active, non-loopback, non-link-local IPv4 addresses sorted with
         * Wi-Fi and private LAN subnets (`192.168.x.x`, `10.x.x.x`) prioritized first.
         */
        fun getLocalIpAddresses(): List<String> {
            return collectCandidates()
                .sortedWith(compareBy<NetworkCandidate> { it.score }.thenByDescending { it.prefixLength })
                .map { it.ip }
                .distinct()
        }

        /**
         * Selects the local IPv4 address whose network adapter and subnet mask best match
         * [remoteAddress], preventing virtual/WSL/VPN adapters from hijacking LAN discovery.
         */
        fun resolveBestLocalIpFor(remoteAddress: InetAddress): String? {
            if (remoteAddress.isLoopbackAddress) {
                return "127.0.0.1"
            }

            val remoteIpv4 = remoteAddress as? Inet4Address ?: return null
            val remoteInt = ipv4ToInt(remoteIpv4.address)
            val candidates = collectCandidates()
                .sortedWith(compareBy<NetworkCandidate> { it.score }.thenByDescending { it.prefixLength })

            val exactSubnetMatch = candidates.firstOrNull { candidate ->
                if (candidate.prefixLength !in 1..32) return@firstOrNull false
                val mask = -1 shl (32 - candidate.prefixLength)
                val localInt = ipv4ToInt(candidate.address.address)
                (localInt and mask) == (remoteInt and mask)
            }
            if (exactSubnetMatch != null) return exactSubnetMatch.ip

            val slash16Match = candidates.firstOrNull { candidate ->
                val mask16 = -1 shl 16
                val localInt = ipv4ToInt(candidate.address.address)
                (localInt and mask16) == (remoteInt and mask16)
            }
            return slash16Match?.ip
        }

        fun resolveComputerName(): String {
            return try {
                InetAddress.getLocalHost().hostName?.takeIf { it.isNotBlank() }
                    ?: System.getenv("COMPUTERNAME")?.takeIf { it.isNotBlank() }
                    ?: "Computador-Windows"
            } catch (_: Exception) {
                System.getenv("COMPUTERNAME")?.takeIf { it.isNotBlank() } ?: "Computador-Windows"
            }
        }

        private fun collectCandidates(): List<NetworkCandidate> {
            val interfaces = try {
                NetworkInterface.getNetworkInterfaces()
            } catch (_: SocketException) {
                null
            } ?: return emptyList()

            val result = mutableListOf<NetworkCandidate>()
            while (interfaces.hasMoreElements()) {
                val iface = interfaces.nextElement()
                val isUp = try {
                    iface.isUp && !iface.isLoopback
                } catch (_: SocketException) {
                    false
                }
                if (!isUp) continue

                val ifaceLabel = "${iface.name} ${iface.displayName}".lowercase()
                for (ifaceAddr in iface.interfaceAddresses) {
                    val addr = ifaceAddr.address as? Inet4Address ?: continue
                    if (addr.isLoopbackAddress ||
                        addr.isLinkLocalAddress ||
                        addr.isAnyLocalAddress ||
                        addr.isMulticastAddress
                    ) {
                        continue
                    }
                    val ip = addr.hostAddress ?: continue
                    result.add(
                        NetworkCandidate(
                            ip = ip,
                            address = addr,
                            prefixLength = ifaceAddr.networkPrefixLength.toInt(),
                            score = computePriorityScore(iface, ifaceAddr, ifaceLabel, ip)
                        )
                    )
                }
            }
            return result
        }

        private fun computePriorityScore(
            iface: NetworkInterface,
            ifaceAddr: InterfaceAddress,
            ifaceLabel: String,
            ip: String
        ): Int {
            var score = 0

            val isVirtualOrTunnel = iface.isVirtual ||
                VIRTUAL_ADAPTER_KEYWORDS.any { ifaceLabel.contains(it) }
            if (isVirtualOrTunnel) {
                score += 200
            }

            if (WIFI_KEYWORDS.any { ifaceLabel.contains(it) }) {
                score -= 30
            } else if (ETHERNET_KEYWORDS.any { ifaceLabel.contains(it) }) {
                score -= 15
            }

            score += when {
                ip.startsWith("192.168.") -> 0
                ip.startsWith("10.") -> 10
                isPrivate172(ifaceAddr.address.address) -> 20
                else -> 40
            }

            return score
        }

        private fun isPrivate172(bytes: ByteArray): Boolean {
            if (bytes.size != 4) return false
            val first = bytes[0].toInt() and 0xFF
            val second = bytes[1].toInt() and 0xFF
            return first == 172 && second in 16..31
        }

        private fun ipv4ToInt(bytes: ByteArray): Int {
            return ((bytes[0].toInt() and 0xFF) shl 24) or
                ((bytes[1].toInt() and 0xFF) shl 16) or
                ((bytes[2].toInt() and 0xFF) shl 8) or
                (bytes[3].toInt() and 0xFF)
        }

        private val WIFI_KEYWORDS = listOf("wi-fi", "wifi", "wlan", "wireless", "802.11")
        private val ETHERNET_KEYWORDS = listOf("ethernet", "eth", "realtek", "intel")
        private val VIRTUAL_ADAPTER_KEYWORDS = listOf(
            "virtual", "vmware", "vbox", "virtualbox", "hyper-v", "vethernet",
            "wsl", "docker", "tun", "tap", "teredo", "bluetooth", "npcap", "hamachi", "tailscale", "zerotier",
            "wireguard", "wintun", "vpn", "nordlynx", "mullvad", "proton"
        )
    }
}

/**
 * Package-level helper returning active local IPv4 addresses sorted with Wi-Fi and LAN subnets first.
 */
fun getLocalIpAddresses(): List<String> = NetworkTouchpadServer.getLocalIpAddresses()
