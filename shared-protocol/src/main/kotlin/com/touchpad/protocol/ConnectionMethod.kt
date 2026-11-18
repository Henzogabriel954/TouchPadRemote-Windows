package com.touchpad.protocol

enum class ConnectionMethod(
    val id: String,
    val displayName: String,
    val description: String,
    val isAvailableNow: Boolean
) {
    NETWORK_LAN(
        id = "NETWORK_LAN",
        displayName = "Via Rede (Wi-Fi / LAN)",
        description = "Conexão UDP de baixa latência na mesma rede local",
        isAvailableNow = true
    ),
    BLUETOOTH(
        id = "BLUETOOTH",
        displayName = "Bluetooth (Em breve)",
        description = "Conexão direta via Bluetooth",
        isAvailableNow = false
    ),
    USB_ADB(
        id = "USB_ADB",
        displayName = "Cabo USB (Em breve)",
        description = "Conexão via cabo USB / ADB",
        isAvailableNow = false
    );

    companion object {
        fun fromId(id: String): ConnectionMethod {
            val normalized = id.trim()
            return entries.firstOrNull { it.id.equals(normalized, ignoreCase = true) } ?: NETWORK_LAN
        }
    }
}
