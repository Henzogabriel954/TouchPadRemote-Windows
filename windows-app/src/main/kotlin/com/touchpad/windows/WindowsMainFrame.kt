package com.touchpad.windows

import com.touchpad.protocol.ConnectionMethod
import com.touchpad.protocol.MouseButton
import com.touchpad.protocol.TouchpadPacket
import java.awt.BasicStroke
import java.awt.BorderLayout
import java.awt.Color
import java.awt.Component
import java.awt.Cursor
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.Font
import java.awt.FontMetrics
import java.awt.Frame
import java.awt.Graphics
import java.awt.Graphics2D
import java.awt.GraphicsEnvironment
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.GridLayout
import java.awt.Insets
import java.awt.Rectangle
import java.awt.RenderingHints
import java.awt.Toolkit
import java.awt.datatransfer.StringSelection
import java.awt.event.KeyAdapter
import java.awt.event.KeyEvent
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import java.awt.event.WindowAdapter
import java.awt.event.WindowEvent
import java.awt.geom.Ellipse2D
import java.awt.geom.RoundRectangle2D
import java.awt.image.BufferedImage
import java.util.Locale
import java.util.concurrent.atomic.AtomicBoolean
import javax.swing.BorderFactory
import javax.swing.Box
import javax.swing.BoxLayout
import javax.swing.JButton
import javax.swing.JFrame
import javax.swing.JLabel
import javax.swing.JPanel
import javax.swing.JScrollPane
import javax.swing.JSlider
import javax.swing.JTextArea
import javax.swing.JTextField
import javax.swing.Scrollable
import javax.swing.SwingUtilities
import javax.swing.Timer
import javax.swing.UIManager
import javax.swing.WindowConstants
import javax.swing.border.EmptyBorder
import javax.swing.plaf.basic.BasicSliderUI

/**
 * Dark-themed Swing desktop window for controlling and monitoring the TouchPad Remote Windows UDP server.
 */
class WindowsMainFrame(
    private val mouseController: WindowsMouseController = WindowsMouseController(),
    private val server: NetworkTouchpadServer = NetworkTouchpadServer(
        mouseController = mouseController,
        requirePairingApproval = true
    )
) : JFrame("TouchPad Remote — Servidor Windows") {

    private val uiRefreshPending = AtomicBoolean(false)

    @Volatile
    private var latestClientIp: String? = null

    @Volatile
    private var latestPacketCount: Long = 0L

    @Volatile
    private var latestActionText: String = "Nenhuma ação recebida ainda"

    private val statusBadge = StatusBadge()
    private val subtitleLabel = JLabel("Use seu celular como touchpad sem fio na mesma rede local")

    private val methodNoticeLabel = JLabel(" ").apply {
        font = uiFont(Font.PLAIN, 12)
        foreground = Theme.ACCENT_CYAN
    }
    private lateinit var methodCards: Map<ConnectionMethod, ConnectionMethodCard>

    private val primaryIpField = JTextField().apply {
        isEditable = false
        border = BorderFactory.createEmptyBorder()
        isOpaque = false
        font = monoFont(Font.BOLD, 25)
        foreground = Theme.ACCENT_CYAN
        caretColor = Theme.ACCENT_CYAN
        selectedTextColor = Theme.BG_CANVAS
        selectionColor = Theme.ACCENT_CYAN
    }
    private val secondaryIpsPanel = JPanel(FlowLayout(FlowLayout.LEFT, 8, 4)).apply {
        isOpaque = false
    }
    private val computerNameValueLabel = JLabel(server.computerName).apply {
        font = uiFont(Font.BOLD, 14)
        foreground = Theme.TEXT_PRIMARY
    }
    private val portValueField = JTextField(TouchpadPacket.DEFAULT_PORT.toString()).apply {
        isEditable = false
        border = BorderFactory.createEmptyBorder()
        isOpaque = false
        font = monoFont(Font.BOLD, 16)
        foreground = Theme.TEXT_PRIMARY
        selectedTextColor = Theme.BG_CANVAS
        selectionColor = Theme.ACCENT_CYAN
    }

    private val copyIpButton = ModernButton(
        text = "Copiar IP",
        baseBg = Theme.BG_ELEVATED,
        hoverBg = Color(0x28, 0x35, 0x48),
        borderColor = Theme.ACCENT_CYAN_BORDER,
        textColor = Theme.ACCENT_CYAN
    ).apply {
        preferredSize = Dimension(122, 38)
        minimumSize = Dimension(122, 38)
    }
    private val refreshIpButton = ModernButton(
        text = "Atualizar Rede",
        baseBg = Theme.BG_SURFACE,
        hoverBg = Theme.BG_ELEVATED,
        borderColor = Theme.BORDER_SUBTLE,
        textColor = Theme.TEXT_SECONDARY
    )
    private val toggleServerButton = ModernButton(
        text = "Parar Servidor",
        baseBg = Color(0x14, 0x33, 0x2B),
        hoverBg = Color(0x19, 0x42, 0x37),
        borderColor = Theme.STATUS_EMERALD,
        textColor = Theme.TEXT_PRIMARY
    ).apply {
        preferredSize = Dimension(180, 42)
        minimumSize = Dimension(180, 42)
    }

    private val sensitivityValueLabel = JLabel(formatSensitivity(mouseController.sensitivity)).apply {
        font = monoFont(Font.BOLD, 13)
        foreground = Theme.ACCENT_CYAN
    }
    private val sensitivitySlider = JSlider(
        (WindowsMouseController.MIN_SENSITIVITY.coerceAtLeast(0.5f) * 10).toInt(),
        (WindowsMouseController.MAX_SENSITIVITY.coerceAtMost(4.0f) * 10).toInt(),
        (mouseController.sensitivity.coerceIn(0.5f, 4.0f) * 10).toInt()
    )

    private val lastClientValueLabel = JLabel("Aguardando celular...").apply {
        font = monoFont(Font.BOLD, 13)
        foreground = Theme.TEXT_PRIMARY
    }
    private val packetCountValueLabel = JLabel("0").apply {
        font = monoFont(Font.BOLD, 13)
        foreground = Theme.TEXT_PRIMARY
    }
    private val lastActionValueLabel = JLabel("Nenhuma ação recebida ainda").apply {
        font = uiFont(Font.BOLD, 13)
        foreground = Theme.TEXT_PRIMARY
    }

    private val serverErrorLabel = JLabel("").apply {
        font = uiFont(Font.PLAIN, 12)
        foreground = Theme.STATUS_STOPPED
        isVisible = false
    }

    private val rootContentPanel: JPanel = object : JPanel(), Scrollable {
        init {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            background = Theme.BG_CANVAS
            border = EmptyBorder(18, 24, 20, 24)
        }

        override fun getPreferredScrollableViewportSize(): Dimension = preferredSize

        override fun getScrollableUnitIncrement(
            visibleRect: Rectangle,
            orientation: Int,
            direction: Int
        ): Int = 16

        override fun getScrollableBlockIncrement(
            visibleRect: Rectangle,
            orientation: Int,
            direction: Int
        ): Int = 64

        override fun getScrollableTracksViewportWidth(): Boolean = true

        override fun getScrollableTracksViewportHeight(): Boolean = false
    }

    private val pairingApprovalCard = RoundedPanel(
        cornerRadius = 14,
        fillColor = Theme.BG_SURFACE,
        borderColor = Theme.BORDER_SUBTLE,
        stretchInBoxLayout = true
    ).apply {
        layout = BoxLayout(this, BoxLayout.Y_AXIS)
        border = EmptyBorder(15, 18, 15, 18)
        alignmentX = Component.LEFT_ALIGNMENT
    }

    private val pairingStatusLabel = JLabel("Aguardando").apply {
        font = uiFont(Font.BOLD, 12)
        foreground = Theme.TEXT_MUTED
    }

    private val pairingListPanel = JPanel().apply {
        layout = BoxLayout(this, BoxLayout.Y_AXIS)
        isOpaque = false
        alignmentX = Component.LEFT_ALIGNMENT
    }

    private val copyFeedbackTimer = Timer(1800) {
        copyIpButton.text = "Copiar IP"
    }.apply {
        isRepeats = false
    }

    init {
        defaultCloseOperation = WindowConstants.DISPOSE_ON_CLOSE
        background = Theme.BG_CANVAS
        iconImages = listOf(createAppIcon(32), createAppIcon(64))
        minimumSize = Dimension(720, 660)
        val screenHeight = try {
            Toolkit.getDefaultToolkit().screenSize.height
        } catch (_: Exception) {
            900
        }
        preferredSize = Dimension(840, (screenHeight - 70).coerceIn(660, 860))

        buildUi()
        attachServerListener()
        attachWindowLifecycle()
        refreshNetworkInfo()
        refreshPairingApprovalUi(server.getPendingPairings(), server.getApprovedClients())

        pack()
        setLocationRelativeTo(null)
        startServerSafely()
    }

    private fun buildUi() {
        rootContentPanel.add(createHeaderSection())
        rootContentPanel.add(Box.createVerticalStrut(14))
        rootContentPanel.add(createMethodSelectorSection())
        rootContentPanel.add(Box.createVerticalStrut(12))
        rootContentPanel.add(createPairingApprovalSection())
        rootContentPanel.add(Box.createVerticalStrut(12))
        rootContentPanel.add(createPairingCard())
        rootContentPanel.add(Box.createVerticalStrut(12))
        rootContentPanel.add(createControlsCard())
        rootContentPanel.add(Box.createVerticalStrut(12))
        rootContentPanel.add(createLiveStatusCard())

        val scrollPane = JScrollPane(rootContentPanel).apply {
            border = BorderFactory.createEmptyBorder()
            viewport.background = Theme.BG_CANVAS
            verticalScrollBar.unitIncrement = 16
            horizontalScrollBarPolicy = JScrollPane.HORIZONTAL_SCROLLBAR_NEVER
        }
        contentPane.layout = BorderLayout()
        contentPane.add(scrollPane, BorderLayout.CENTER)
    }

    private fun createHeaderSection(): JPanel {
        return JPanel(BorderLayout(16, 0)).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
            maximumSize = Dimension(Int.MAX_VALUE, 68)

            val titleStack = JPanel().apply {
                layout = BoxLayout(this, BoxLayout.Y_AXIS)
                isOpaque = false

                val titleLabel = JLabel("TouchPad Remote — Servidor Windows").apply {
                    font = uiFont(Font.BOLD, 21)
                    foreground = Theme.TEXT_PRIMARY
                    alignmentX = Component.LEFT_ALIGNMENT
                }
                subtitleLabel.apply {
                    font = uiFont(Font.PLAIN, 13)
                    foreground = Theme.TEXT_SECONDARY
                    alignmentX = Component.LEFT_ALIGNMENT
                }
                add(titleLabel)
                add(Box.createVerticalStrut(4))
                add(subtitleLabel)
            }

            val badgeHolder = JPanel(GridBagLayout()).apply {
                isOpaque = false
                add(statusBadge)
            }

            add(titleStack, BorderLayout.CENTER)
            add(badgeHolder, BorderLayout.EAST)
        }
    }

    private fun createMethodSelectorSection(): JPanel {
        val container = RoundedPanel(
            cornerRadius = 14,
            fillColor = Theme.BG_SURFACE,
            borderColor = Theme.BORDER_SUBTLE,
            stretchInBoxLayout = true
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(14, 16, 14, 16)
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val heading = JLabel("Método de Conexão").apply {
            font = uiFont(Font.BOLD, 14)
            foreground = Theme.TEXT_PRIMARY
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val cardsRow = JPanel(GridLayout(1, 3, 12, 0)).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val lanCard = ConnectionMethodCard(
            method = ConnectionMethod.NETWORK_LAN,
            title = "Via Rede (Wi-Fi / LAN)",
            description = "Conexão rápida pela rede Wi-Fi ou cabo Ethernet local.",
            badgeText = "Ativo",
            isSelected = true
        ) {
            onMethodSelected(ConnectionMethod.NETWORK_LAN)
        }

        val bluetoothCard = ConnectionMethodCard(
            method = ConnectionMethod.BLUETOOTH,
            title = "Bluetooth",
            description = "Pareamento direto sem roteador.",
            badgeText = "(Em breve)",
            isSelected = false
        ) {
            onMethodSelected(ConnectionMethod.BLUETOOTH)
        }

        val usbCard = ConnectionMethodCard(
            method = ConnectionMethod.USB_ADB,
            title = "Cabo USB",
            description = "Conexão com baixa latência via ADB.",
            badgeText = "(Em breve)",
            isSelected = false
        ) {
            onMethodSelected(ConnectionMethod.USB_ADB)
        }

        methodCards = mapOf(
            ConnectionMethod.NETWORK_LAN to lanCard,
            ConnectionMethod.BLUETOOTH to bluetoothCard,
            ConnectionMethod.USB_ADB to usbCard
        )

        cardsRow.add(lanCard)
        cardsRow.add(bluetoothCard)
        cardsRow.add(usbCard)

        methodNoticeLabel.alignmentX = Component.LEFT_ALIGNMENT

        container.add(heading)
        container.add(Box.createVerticalStrut(10))
        container.add(cardsRow)
        container.add(Box.createVerticalStrut(8))
        container.add(methodNoticeLabel)
        return container
    }

    private fun createPairingApprovalSection(): JPanel {
        val headerRow = JPanel(BorderLayout()).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
            add(JLabel("Pareamento do Celular").apply {
                font = uiFont(Font.BOLD, 14)
                foreground = Theme.TEXT_PRIMARY
            }, BorderLayout.WEST)
            add(pairingStatusLabel, BorderLayout.EAST)
        }

        pairingApprovalCard.add(headerRow)
        pairingApprovalCard.add(Box.createVerticalStrut(10))
        pairingApprovalCard.add(pairingListPanel)
        return pairingApprovalCard
    }

    private fun refreshPairingApprovalUi(
        pending: List<PairingClientInfo>,
        approved: List<PairingClientInfo>
    ) {
        pairingListPanel.removeAll()

        when {
            pending.isNotEmpty() -> {
                pairingApprovalCard.updateSurface(
                    fill = Color(0x13, 0x1D, 0x31),
                    border = Theme.ACCENT_CYAN,
                    strokeWidth = 1.5f
                )
                pairingStatusLabel.text = if (pending.size == 1) {
                    "Solicitação pendente"
                } else {
                    "${pending.size} solicitações pendentes"
                }
                pairingStatusLabel.foreground = Theme.ACCENT_CYAN
            }

            approved.isNotEmpty() -> {
                pairingApprovalCard.updateSurface(
                    fill = Theme.BG_SURFACE,
                    border = Theme.STATUS_EMERALD,
                    strokeWidth = 1.2f
                )
                pairingStatusLabel.text = if (approved.size == 1) {
                    "1 celular autorizado"
                } else {
                    "${approved.size} celulares autorizados"
                }
                pairingStatusLabel.foreground = Theme.STATUS_EMERALD
            }

            else -> {
                pairingApprovalCard.updateSurface(
                    fill = Theme.BG_SURFACE,
                    border = Theme.BORDER_SUBTLE,
                    strokeWidth = 1f
                )
                pairingStatusLabel.text = "Aguardando"
                pairingStatusLabel.foreground = Theme.TEXT_MUTED
            }
        }

        if (pending.isEmpty() && approved.isEmpty()) {
            val emptyBox = RoundedPanel(
                cornerRadius = 10,
                fillColor = Theme.BG_ELEVATED,
                borderColor = Theme.BORDER_SUBTLE,
                stretchInBoxLayout = true
            ).apply {
                layout = BorderLayout()
                border = EmptyBorder(12, 14, 12, 14)
                alignmentX = Component.LEFT_ALIGNMENT

                val messageArea = JTextArea(
                    "Aguardando solicitação de pareamento do celular. Quando você tocar no quadrado no celular, o pedido aparecerá aqui para aceitar.",
                    2,
                    48
                ).apply {
                    isEditable = false
                    isFocusable = false
                    isOpaque = false
                    lineWrap = true
                    wrapStyleWord = true
                    font = uiFont(Font.PLAIN, 12)
                    foreground = Theme.TEXT_SECONDARY
                    border = EmptyBorder(0, 0, 0, 0)
                }
                add(messageArea, BorderLayout.CENTER)
            }
            pairingListPanel.add(emptyBox)
        } else {
            pending.forEachIndexed { index, client ->
                if (index > 0) pairingListPanel.add(Box.createVerticalStrut(8))
                pairingListPanel.add(createPendingPairingRow(client))
            }
            if (pending.isNotEmpty() && approved.isNotEmpty()) {
                pairingListPanel.add(Box.createVerticalStrut(10))
            }
            approved.forEachIndexed { index, client ->
                if (index > 0) pairingListPanel.add(Box.createVerticalStrut(8))
                pairingListPanel.add(createApprovedPairingRow(client))
            }
        }

        pairingListPanel.revalidate()
        pairingApprovalCard.revalidate()
        rootContentPanel.revalidate()
        rootContentPanel.repaint()
    }

    private fun createPendingPairingRow(client: PairingClientInfo): JPanel {
        return RoundedPanel(
            cornerRadius = 12,
            fillColor = Color(0x16, 0x26, 0x3D),
            borderColor = Theme.ACCENT_CYAN,
            borderStrokeWidth = 1.2f,
            stretchInBoxLayout = true
        ).apply {
            layout = BorderLayout(14, 8)
            border = EmptyBorder(12, 14, 12, 14)
            alignmentX = Component.LEFT_ALIGNMENT

            val infoCol = JPanel().apply {
                layout = BoxLayout(this, BoxLayout.Y_AXIS)
                isOpaque = false

                add(JLabel("Solicitação de pareamento recebida").apply {
                    font = uiFont(Font.BOLD, 11)
                    foreground = Theme.ACCENT_CYAN
                    alignmentX = Component.LEFT_ALIGNMENT
                })
                add(Box.createVerticalStrut(3))
                add(JLabel(client.clientName).apply {
                    font = uiFont(Font.BOLD, 15)
                    foreground = Theme.TEXT_PRIMARY
                    alignmentX = Component.LEFT_ALIGNMENT
                })
                add(Box.createVerticalStrut(2))
                add(JLabel("IP do celular: ${client.ipAddress}").apply {
                    font = monoFont(Font.PLAIN, 12)
                    foreground = Theme.TEXT_SECONDARY
                    alignmentX = Component.LEFT_ALIGNMENT
                })
            }

            val acceptBtn = ModernButton(
                text = "Aceitar Pareamento",
                baseBg = Color(0x0E, 0x35, 0x29),
                hoverBg = Color(0x13, 0x47, 0x37),
                borderColor = Theme.STATUS_EMERALD,
                textColor = Color(0xD1, 0xFA, 0xE5)
            ).apply {
                addActionListener {
                    server.approvePairing(client.ipAddress)
                }
            }

            val rejectBtn = ModernButton(
                text = "Recusar",
                baseBg = Color(0x33, 0x16, 0x1E),
                hoverBg = Color(0x47, 0x1D, 0x27),
                borderColor = Theme.STATUS_STOPPED,
                textColor = Color(0xFE, 0xCA, 0xCA)
            ).apply {
                addActionListener {
                    server.rejectPairing(client.ipAddress)
                }
            }

            val actionsRow = JPanel(FlowLayout(FlowLayout.RIGHT, 8, 0)).apply {
                isOpaque = false
                add(acceptBtn)
                add(rejectBtn)
            }

            val actionsHolder = JPanel(GridBagLayout()).apply {
                isOpaque = false
                add(actionsRow)
            }

            add(infoCol, BorderLayout.CENTER)
            add(actionsHolder, BorderLayout.EAST)
        }
    }

    private fun createApprovedPairingRow(client: PairingClientInfo): JPanel {
        return RoundedPanel(
            cornerRadius = 10,
            fillColor = Theme.BG_ELEVATED,
            borderColor = Color(0x16, 0x5B, 0x45),
            stretchInBoxLayout = true
        ).apply {
            layout = BorderLayout(12, 0)
            border = EmptyBorder(10, 14, 10, 14)
            alignmentX = Component.LEFT_ALIGNMENT

            val dotPanel = object : JPanel() {
                override fun paintComponent(g: Graphics) {
                    super.paintComponent(g)
                    val g2 = g.create() as Graphics2D
                    g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
                    g2.color = Theme.STATUS_EMERALD
                    g2.fill(Ellipse2D.Float(0f, (height - 10f) / 2f, 10f, 10f))
                    g2.dispose()
                }
            }.apply {
                isOpaque = false
                preferredSize = Dimension(14, 24)
            }

            val textCol = JPanel().apply {
                layout = BoxLayout(this, BoxLayout.Y_AXIS)
                isOpaque = false
                add(JLabel("${client.clientName} (${client.ipAddress})").apply {
                    font = uiFont(Font.BOLD, 13)
                    foreground = Theme.TEXT_PRIMARY
                    alignmentX = Component.LEFT_ALIGNMENT
                })
                add(Box.createVerticalStrut(2))
                add(JLabel("Pareado e autorizado para controlar este computador").apply {
                    font = uiFont(Font.PLAIN, 11)
                    foreground = Theme.STATUS_EMERALD
                    alignmentX = Component.LEFT_ALIGNMENT
                })
            }

            val leftGroup = JPanel(BorderLayout(10, 0)).apply {
                isOpaque = false
                add(dotPanel, BorderLayout.WEST)
                add(textCol, BorderLayout.CENTER)
            }

            val disconnectBtn = ModernButton(
                text = "Desconectar",
                baseBg = Theme.BG_SURFACE,
                hoverBg = Color(0x33, 0x16, 0x1E),
                borderColor = Theme.BORDER_SUBTLE,
                textColor = Theme.TEXT_SECONDARY,
                compact = true
            ).apply {
                addActionListener {
                    server.revokePairing(client.ipAddress)
                }
            }

            val btnHolder = JPanel(GridBagLayout()).apply {
                isOpaque = false
                add(disconnectBtn)
            }

            add(leftGroup, BorderLayout.CENTER)
            add(btnHolder, BorderLayout.EAST)
        }
    }

    private fun createPairingCard(): JPanel {
        val card = RoundedPanel(
            cornerRadius = 14,
            fillColor = Theme.BG_SURFACE,
            borderColor = Theme.BORDER_SUBTLE,
            stretchInBoxLayout = true
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(16, 18, 16, 18)
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val headerRow = JPanel(BorderLayout()).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
            val title = JLabel("Dados para Cadastrar no Celular").apply {
                font = uiFont(Font.BOLD, 14)
                foreground = Theme.TEXT_PRIMARY
            }
            add(title, BorderLayout.WEST)
            add(refreshIpButton.apply {
                addActionListener { refreshNetworkInfo() }
            }, BorderLayout.EAST)
        }

        val ipBlock = RoundedPanel(
            cornerRadius = 12,
            fillColor = Theme.BG_ELEVATED,
            borderColor = Theme.ACCENT_CYAN_BORDER,
            stretchInBoxLayout = true
        ).apply {
            layout = BorderLayout(12, 6)
            border = EmptyBorder(12, 14, 12, 14)
            alignmentX = Component.LEFT_ALIGNMENT

            val leftCol = JPanel().apply {
                layout = BoxLayout(this, BoxLayout.Y_AXIS)
                isOpaque = false

                val ipCaption = JLabel("Endereço IP na Rede Local").apply {
                    font = uiFont(Font.PLAIN, 12)
                    foreground = Theme.TEXT_SECONDARY
                    alignmentX = Component.LEFT_ALIGNMENT
                }
                primaryIpField.alignmentX = Component.LEFT_ALIGNMENT
                secondaryIpsPanel.alignmentX = Component.LEFT_ALIGNMENT

                add(ipCaption)
                add(Box.createVerticalStrut(4))
                add(primaryIpField)
                add(secondaryIpsPanel)
            }

            val buttonBox = JPanel(GridBagLayout()).apply {
                isOpaque = false
                copyIpButton.addActionListener { copyPrimaryIp() }
                add(copyIpButton)
            }

            add(leftCol, BorderLayout.CENTER)
            add(buttonBox, BorderLayout.EAST)
        }

        val metaGrid = JPanel(GridLayout(1, 2, 12, 0)).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val computerTile = RoundedPanel(
            cornerRadius = 10,
            fillColor = Theme.BG_ELEVATED,
            borderColor = Theme.BORDER_SUBTLE
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(10, 14, 10, 14)
            add(JLabel("Nome do Computador").apply {
                font = uiFont(Font.PLAIN, 12)
                foreground = Theme.TEXT_SECONDARY
                alignmentX = Component.LEFT_ALIGNMENT
            })
            add(Box.createVerticalStrut(4))
            add(computerNameValueLabel.apply { alignmentX = Component.LEFT_ALIGNMENT })
        }

        val portTile = RoundedPanel(
            cornerRadius = 10,
            fillColor = Theme.BG_ELEVATED,
            borderColor = Theme.BORDER_SUBTLE
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(10, 14, 10, 14)
            add(JLabel("Porta UDP").apply {
                font = uiFont(Font.PLAIN, 12)
                foreground = Theme.TEXT_SECONDARY
                alignmentX = Component.LEFT_ALIGNMENT
            })
            add(Box.createVerticalStrut(4))
            add(portValueField.apply { alignmentX = Component.LEFT_ALIGNMENT })
        }

        metaGrid.add(computerTile)
        metaGrid.add(portTile)

        card.add(headerRow)
        card.add(Box.createVerticalStrut(12))
        card.add(ipBlock)
        card.add(Box.createVerticalStrut(10))
        card.add(metaGrid)
        return card
    }

    private fun createControlsCard(): JPanel {
        val card = RoundedPanel(
            cornerRadius = 14,
            fillColor = Theme.BG_SURFACE,
            borderColor = Theme.BORDER_SUBTLE,
            stretchInBoxLayout = true
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(16, 18, 16, 18)
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val title = JLabel("Controles").apply {
            font = uiFont(Font.BOLD, 14)
            foreground = Theme.TEXT_PRIMARY
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val controlsRow = JPanel(GridBagLayout()).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
        }

        toggleServerButton.addActionListener {
            if (server.isRunning) {
                server.stop()
            } else {
                startServerSafely()
            }
        }

        val sliderPanel = JPanel().apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            isOpaque = false

            val sliderHeader = JPanel(BorderLayout()).apply {
                isOpaque = false
                alignmentX = Component.LEFT_ALIGNMENT
                add(JLabel("Sensibilidade do Touchpad").apply {
                    font = uiFont(Font.PLAIN, 13)
                    foreground = Theme.TEXT_SECONDARY
                }, BorderLayout.WEST)
                add(sensitivityValueLabel, BorderLayout.EAST)
            }

            sensitivitySlider.apply {
                isOpaque = false
                alignmentX = Component.LEFT_ALIGNMENT
                setUI(ModernDarkSliderUI(this))
                addChangeListener {
                    val newSensitivity = value / 10.0f
                    mouseController.sensitivity = newSensitivity
                    sensitivityValueLabel.text = formatSensitivity(newSensitivity)
                }
            }

            val rangeLabels = JPanel(BorderLayout()).apply {
                isOpaque = false
                alignmentX = Component.LEFT_ALIGNMENT
                add(JLabel("0.5x (Preciso)").apply {
                    font = uiFont(Font.PLAIN, 11)
                    foreground = Theme.TEXT_MUTED
                }, BorderLayout.WEST)
                add(JLabel("4.0x (Rápido)").apply {
                    font = uiFont(Font.PLAIN, 11)
                    foreground = Theme.TEXT_MUTED
                }, BorderLayout.EAST)
            }

            add(sliderHeader)
            add(Box.createVerticalStrut(4))
            add(sensitivitySlider)
            add(rangeLabels)
        }

        val gbcBtn = GridBagConstraints().apply {
            gridx = 0
            gridy = 0
            weightx = 0.0
            anchor = GridBagConstraints.WEST
            insets = Insets(0, 0, 0, 22)
        }
        val gbcSlider = GridBagConstraints().apply {
            gridx = 1
            gridy = 0
            weightx = 1.0
            fill = GridBagConstraints.HORIZONTAL
        }

        controlsRow.add(toggleServerButton, gbcBtn)
        controlsRow.add(sliderPanel, gbcSlider)

        serverErrorLabel.alignmentX = Component.LEFT_ALIGNMENT

        card.add(title)
        card.add(Box.createVerticalStrut(12))
        card.add(controlsRow)
        card.add(Box.createVerticalStrut(6))
        card.add(serverErrorLabel)
        return card
    }

    private fun createLiveStatusCard(): JPanel {
        val card = RoundedPanel(
            cornerRadius = 14,
            fillColor = Theme.BG_SURFACE,
            borderColor = Theme.BORDER_SUBTLE,
            stretchInBoxLayout = true
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(16, 18, 16, 18)
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val title = JLabel("Status em Tempo Real").apply {
            font = uiFont(Font.BOLD, 14)
            foreground = Theme.TEXT_PRIMARY
            alignmentX = Component.LEFT_ALIGNMENT
        }

        val metricsGrid = JPanel(GridLayout(1, 3, 12, 0)).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
        }

        metricsGrid.add(createMetricTile("Último celular conectado", lastClientValueLabel))
        metricsGrid.add(createMetricTile("Comandos recebidos", packetCountValueLabel))
        metricsGrid.add(createMetricTile("Última ação executada", lastActionValueLabel))

        card.add(title)
        card.add(Box.createVerticalStrut(10))
        card.add(metricsGrid)
        return card
    }

    private fun createMetricTile(caption: String, valueLabel: JLabel): JPanel {
        return RoundedPanel(
            cornerRadius = 10,
            fillColor = Theme.BG_ELEVATED,
            borderColor = Theme.BORDER_SUBTLE
        ).apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = EmptyBorder(10, 12, 10, 12)
            add(JLabel(caption).apply {
                font = uiFont(Font.PLAIN, 12)
                foreground = Theme.TEXT_SECONDARY
                alignmentX = Component.LEFT_ALIGNMENT
            })
            add(Box.createVerticalStrut(5))
            add(valueLabel.apply { alignmentX = Component.LEFT_ALIGNMENT })
        }
    }

    private fun onMethodSelected(method: ConnectionMethod) {
        if (method.isAvailableNow) {
            methodNoticeLabel.text = " "
            methodCards.forEach { (key, card) ->
                card.setSelectedState(key == method)
            }
        } else {
            val label = when (method) {
                ConnectionMethod.BLUETOOTH -> "Bluetooth"
                ConnectionMethod.USB_ADB -> "Cabo USB"
                ConnectionMethod.NETWORK_LAN -> "Via Rede (Wi-Fi / LAN)"
            }
            methodNoticeLabel.text =
                "$label estará disponível em breve. O modo Via Rede (Wi-Fi / LAN) continua ativo."
            methodCards.forEach { (key, card) ->
                card.setSelectedState(key == ConnectionMethod.NETWORK_LAN)
            }
        }
    }

    private fun refreshNetworkInfo() {
        val ips = NetworkTouchpadServer.getLocalIpAddresses()
        val primaryIp = ips.firstOrNull() ?: "Nenhum IP local detectado"
        primaryIpField.text = primaryIp
        copyIpButton.isEnabled = ips.isNotEmpty()

        secondaryIpsPanel.removeAll()
        secondaryIpsPanel.isVisible = ips.size != 1
        if (ips.size > 1) {
            secondaryIpsPanel.add(JLabel("Outros IPs:").apply {
                font = uiFont(Font.PLAIN, 11)
                foreground = Theme.TEXT_MUTED
            })
            ips.drop(1).take(4).forEach { extraIp ->
                val chip = ModernButton(
                    text = extraIp,
                    baseBg = Theme.BG_SURFACE,
                    hoverBg = Color(0x28, 0x35, 0x48),
                    borderColor = Theme.BORDER_SUBTLE,
                    textColor = Theme.TEXT_SECONDARY,
                    compact = true
                ).apply {
                    font = monoFont(Font.PLAIN, 11)
                    toolTipText = "Clique para usar e copiar este IP"
                    addActionListener {
                        primaryIpField.text = extraIp
                        copyPrimaryIp()
                    }
                }
                secondaryIpsPanel.add(chip)
            }
        } else if (ips.isEmpty()) {
            secondaryIpsPanel.add(JLabel("Conecte o computador ao Wi-Fi ou cabo Ethernet e clique em Atualizar Rede.").apply {
                font = uiFont(Font.PLAIN, 11)
                foreground = Theme.STATUS_AMBER
            })
        }
        secondaryIpsPanel.revalidate()
        secondaryIpsPanel.repaint()
        rootContentPanel.revalidate()
        rootContentPanel.repaint()
    }

    private fun copyPrimaryIp() {
        val ipText = primaryIpField.text.trim()
        if (ipText.isEmpty() || ipText.startsWith("Nenhum")) return

        try {
            Toolkit.getDefaultToolkit().systemClipboard.setContents(StringSelection(ipText), null)
            copyIpButton.text = "Copiado!"
            copyFeedbackTimer.restart()
        } catch (_: Exception) {
            copyIpButton.text = "Erro ao copiar"
            copyFeedbackTimer.restart()
        }
    }

    private fun startServerSafely() {
        serverErrorLabel.isVisible = false
        serverErrorLabel.text = ""
        try {
            server.start()
            updateServerStateUi(running = server.isRunning, port = server.actualPort)
        } catch (ex: Exception) {
            updateServerStateUi(running = false, port = server.port)
            serverErrorLabel.text =
                "Não foi possível abrir a porta UDP ${server.port}: ${ex.localizedMessage ?: "porta em uso"}"
            serverErrorLabel.isVisible = true
            rootContentPanel.revalidate()
            rootContentPanel.repaint()
        }
    }

    private fun attachServerListener() {
        server.listener = object : ServerListener {
            override fun onServerStateChanged(running: Boolean, port: Int) {
                SwingUtilities.invokeLater {
                    if (!isDisplayable) return@invokeLater
                    updateServerStateUi(running, port)
                }
            }

            override fun onPacketReceived(packet: TouchpadPacket, clientIp: String, totalPackets: Long) {
                latestClientIp = clientIp
                latestPacketCount = totalPackets
                latestActionText = describePacket(packet)
                scheduleStatusRefresh()
            }

            override fun onDiscoveryRequested(clientName: String, clientIp: String, respondedIp: String) {
                latestClientIp = clientIp
                latestActionText = "Descoberta de $clientName"
                scheduleStatusRefresh()
            }

            override fun onPingReceived(clientIp: String) {
                latestClientIp = clientIp
                latestActionText = "Ping respondido"
                scheduleStatusRefresh()
            }

            override fun onPairingRequested(client: PairingClientInfo) {
                latestClientIp = client.ipAddress
                latestActionText = "Pedido de pareamento: ${client.clientName}"
                scheduleStatusRefresh()
                SwingUtilities.invokeLater {
                    if (!isDisplayable) return@invokeLater
                    if ((extendedState and Frame.ICONIFIED) != 0) {
                        extendedState = Frame.NORMAL
                    }
                    isVisible = true
                    val wasAlwaysOnTop = isAlwaysOnTop
                    isAlwaysOnTop = true
                    toFront()
                    isAlwaysOnTop = wasAlwaysOnTop
                    requestFocus()
                }
            }

            override fun onPairingStateChanged(
                pending: List<PairingClientInfo>,
                approved: List<PairingClientInfo>
            ) {
                SwingUtilities.invokeLater {
                    if (!isDisplayable) return@invokeLater
                    refreshPairingApprovalUi(pending, approved)
                }
            }

            override fun onServerError(message: String, cause: Throwable?) {
                SwingUtilities.invokeLater {
                    if (!isDisplayable) return@invokeLater
                    serverErrorLabel.text = message
                    serverErrorLabel.isVisible = true
                    rootContentPanel.revalidate()
                    rootContentPanel.repaint()
                }
            }
        }
    }

    private fun scheduleStatusRefresh() {
        if (uiRefreshPending.compareAndSet(false, true)) {
            SwingUtilities.invokeLater {
                uiRefreshPending.set(false)
                if (!isDisplayable) return@invokeLater
                lastClientValueLabel.text = latestClientIp ?: "Aguardando celular..."
                packetCountValueLabel.text = latestPacketCount.toString()
                lastActionValueLabel.text = latestActionText
            }
        }
    }

    private fun updateServerStateUi(running: Boolean, port: Int) {
        statusBadge.setOnline(running)
        if (port > 0) {
            portValueField.text = port.toString()
        }
        if (running) {
            serverErrorLabel.isVisible = false
            toggleServerButton.text = "Parar Servidor"
            toggleServerButton.updatePalette(
                baseBg = Color(0x33, 0x16, 0x1E),
                hoverBg = Color(0x47, 0x1D, 0x27),
                borderColor = Theme.STATUS_STOPPED,
                textColor = Color(0xFE, 0xCA, 0xCA)
            )
        } else {
            toggleServerButton.text = "Iniciar Servidor"
            toggleServerButton.updatePalette(
                baseBg = Color(0x0E, 0x35, 0x29),
                hoverBg = Color(0x13, 0x47, 0x37),
                borderColor = Theme.STATUS_EMERALD,
                textColor = Color(0xD1, 0xFA, 0xE5)
            )
        }
        rootContentPanel.revalidate()
        rootContentPanel.repaint()
    }

    private fun shutdownFrameResources() {
        copyFeedbackTimer.stop()
        server.listener = null
        server.stop()
    }

    private fun attachWindowLifecycle() {
        addWindowListener(object : WindowAdapter() {
            override fun windowClosing(e: WindowEvent?) {
                shutdownFrameResources()
            }

            override fun windowClosed(e: WindowEvent?) {
                shutdownFrameResources()
            }
        })
    }

    private fun describePacket(packet: TouchpadPacket): String = when (packet) {
        is TouchpadPacket.Move -> "Mover cursor"
        is TouchpadPacket.Click -> when (packet.button) {
            MouseButton.LEFT -> "Clique esquerdo"
            MouseButton.RIGHT -> "Clique direito"
            MouseButton.MIDDLE -> "Clique do meio"
        }
        is TouchpadPacket.ButtonState -> {
            val action = if (packet.pressed) "Segurar" else "Soltar"
            val buttonName = when (packet.button) {
                MouseButton.LEFT -> "esquerdo"
                MouseButton.RIGHT -> "direito"
                MouseButton.MIDDLE -> "do meio"
            }
            "$action botão $buttonName"
        }
        is TouchpadPacket.Scroll -> if (packet.deltaY >= 0f) "Rolagem para baixo" else "Rolagem para cima"
        is TouchpadPacket.DiscoverRequest -> "Solicitação de descoberta"
        is TouchpadPacket.DiscoverResponse -> "Resposta de descoberta"
        is TouchpadPacket.Ping -> "Ping"
        is TouchpadPacket.Pong -> "Pong"
        is TouchpadPacket.PairRequest -> "Pedido de pareamento"
        is TouchpadPacket.PairResponse -> "Resposta de pareamento"
    }

    private fun formatSensitivity(value: Float): String =
        String.format(Locale.US, "%.1fx", value)

    companion object {
        private val availableFonts: Set<String> by lazy {
            try {
                GraphicsEnvironment.getLocalGraphicsEnvironment().availableFontFamilyNames.toSet()
            } catch (_: Exception) {
                emptySet()
            }
        }

        internal fun uiFont(style: Int, size: Int): Font {
            val family = if ("Segoe UI" in availableFonts) "Segoe UI" else Font.SANS_SERIF
            return Font(family, style, size)
        }

        internal fun monoFont(style: Int, size: Int): Font {
            val family = when {
                "Consolas" in availableFonts -> "Consolas"
                "Cascadia Mono" in availableFonts -> "Cascadia Mono"
                else -> Font.MONOSPACED
            }
            return Font(family, style, size)
        }

        private fun createAppIcon(size: Int): BufferedImage {
            val image = BufferedImage(size, size, BufferedImage.TYPE_INT_ARGB)
            val g2 = image.createGraphics()
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)

            val pad = size * 0.08f
            val rectSize = size - pad * 2f
            g2.color = Theme.BG_CANVAS
            g2.fill(RoundRectangle2D.Float(pad, pad, rectSize, rectSize, size * 0.28f, size * 0.28f))

            g2.color = Theme.ACCENT_CYAN
            g2.stroke = BasicStroke((size * 0.07f).coerceAtLeast(1.5f))
            g2.draw(RoundRectangle2D.Float(pad, pad, rectSize, rectSize, size * 0.28f, size * 0.28f))

            val dotSize = size * 0.24f
            g2.fill(Ellipse2D.Float((size - dotSize) / 2f, size * 0.30f, dotSize, dotSize))

            g2.color = Theme.STATUS_EMERALD
            val barW = size * 0.44f
            val barH = size * 0.12f
            g2.fill(RoundRectangle2D.Float((size - barW) / 2f, size * 0.65f, barW, barH, barH, barH))

            g2.dispose()
            return image
        }
    }
}

private object Theme {
    val BG_CANVAS = Color(0x0B, 0x11, 0x20)
    val BG_SURFACE = Color(0x11, 0x18, 0x27)
    val BG_ELEVATED = Color(0x1E, 0x29, 0x3B)
    val BORDER_SUBTLE = Color(0x2A, 0x38, 0x4E)
    val ACCENT_CYAN = Color(0x38, 0xBD, 0xF8)
    val ACCENT_CYAN_BORDER = Color(0x1E, 0x60, 0x88)
    val STATUS_EMERALD = Color(0x10, 0xB9, 0x81)
    val STATUS_EMERALD_BG = Color(0x0D, 0x2D, 0x24)
    val STATUS_AMBER = Color(0xF5, 0x9E, 0x0B)
    val STATUS_STOPPED = Color(0xEF, 0x44, 0x44)
    val STATUS_STOPPED_BG = Color(0x2A, 0x15, 0x1B)
    val TEXT_PRIMARY = Color(0xF8, 0xFA, 0xFC)
    val TEXT_SECONDARY = Color(0x94, 0xA3, 0xB8)
    val TEXT_MUTED = Color(0x64, 0x74, 0x8B)
}

private open class RoundedPanel(
    private val cornerRadius: Int,
    protected var fillColor: Color,
    protected var borderColor: Color? = null,
    private var borderStrokeWidth: Float = 1f,
    private val stretchInBoxLayout: Boolean = false
) : JPanel() {
    init {
        isOpaque = false
    }

    fun updateSurface(fill: Color, border: Color?, strokeWidth: Float = borderStrokeWidth) {
        fillColor = fill
        borderColor = border
        borderStrokeWidth = strokeWidth
        repaint()
    }

    override fun getMaximumSize(): Dimension {
        val max = super.getMaximumSize()
        return if (stretchInBoxLayout) {
            Dimension(Int.MAX_VALUE, preferredSize.height)
        } else {
            max
        }
    }

    override fun paintComponent(g: Graphics) {
        val g2 = g.create() as Graphics2D
        g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)

        val inset = borderStrokeWidth / 2f
        val rect = RoundRectangle2D.Float(
            inset,
            inset,
            (width - borderStrokeWidth).coerceAtLeast(1f),
            (height - borderStrokeWidth).coerceAtLeast(1f),
            cornerRadius.toFloat(),
            cornerRadius.toFloat()
        )
        g2.color = fillColor
        g2.fill(rect)

        borderColor?.let { strokeColor ->
            g2.color = strokeColor
            g2.stroke = BasicStroke(borderStrokeWidth)
            g2.draw(rect)
        }
        g2.dispose()
        super.paintComponent(g)
    }
}

private class StatusBadge : RoundedPanel(
    cornerRadius = 20,
    fillColor = Theme.STATUS_STOPPED_BG,
    borderColor = Theme.STATUS_STOPPED
) {
    private val dotPanel = object : JPanel() {
        override fun paintComponent(g: Graphics) {
            super.paintComponent(g)
            val g2 = g.create() as Graphics2D
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            g2.color = dotColor
            g2.fill(Ellipse2D.Float(0f, (height - 9f) / 2f, 9f, 9f))
            g2.dispose()
        }
    }.apply {
        isOpaque = false
        preferredSize = Dimension(10, 16)
    }

    private var dotColor: Color = Theme.STATUS_STOPPED
    private val label = JLabel("Parado").apply {
        font = WindowsMainFrame.uiFont(Font.BOLD, 12)
        foreground = Theme.TEXT_PRIMARY
    }

    init {
        layout = FlowLayout(FlowLayout.CENTER, 7, 5)
        border = EmptyBorder(1, 8, 1, 10)
        add(dotPanel)
        add(label)
    }

    fun setOnline(online: Boolean) {
        if (online) {
            fillColor = Theme.STATUS_EMERALD_BG
            borderColor = Theme.STATUS_EMERALD
            dotColor = Theme.STATUS_EMERALD
            label.text = "Ativo na Rede"
            label.foreground = Color(0xD1, 0xFA, 0xE5)
        } else {
            fillColor = Theme.STATUS_STOPPED_BG
            borderColor = Theme.STATUS_STOPPED
            dotColor = Theme.STATUS_STOPPED
            label.text = "Parado"
            label.foreground = Color(0xFE, 0xCA, 0xCA)
        }
        repaint()
    }
}

private class ConnectionMethodCard(
    val method: ConnectionMethod,
    title: String,
    description: String,
    badgeText: String,
    isSelected: Boolean,
    onClick: () -> Unit
) : RoundedPanel(
    cornerRadius = 12,
    fillColor = Theme.BG_ELEVATED,
    borderColor = if (isSelected) Theme.ACCENT_CYAN else Theme.BORDER_SUBTLE,
    borderStrokeWidth = if (isSelected) 1.5f else 1f
) {
    private var selected: Boolean = isSelected
    private var hovered: Boolean = false

    private val indicatorDot = object : JPanel() {
        override fun paintComponent(g: Graphics) {
            super.paintComponent(g)
            val g2 = g.create() as Graphics2D
            g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)
            if (selected) {
                g2.color = Theme.ACCENT_CYAN
                g2.fill(Ellipse2D.Float(1f, 3f, 10f, 10f))
            } else {
                g2.color = Theme.TEXT_MUTED
                g2.stroke = BasicStroke(1.5f)
                g2.draw(Ellipse2D.Float(1f, 3f, 10f, 10f))
            }
            g2.dispose()
        }
    }.apply {
        isOpaque = false
        preferredSize = Dimension(14, 16)
    }

    private val badgeLabel = JLabel(badgeText).apply {
        font = WindowsMainFrame.uiFont(Font.BOLD, 11)
        foreground = if (method.isAvailableNow) Theme.STATUS_EMERALD else Theme.TEXT_MUTED
    }

    private val titleLabel = JLabel(title).apply {
        font = WindowsMainFrame.uiFont(Font.BOLD, 13)
        foreground = if (method.isAvailableNow) Theme.TEXT_PRIMARY else Theme.TEXT_SECONDARY
        alignmentX = Component.LEFT_ALIGNMENT
    }

    private val descArea = JTextArea(description, 2, 18).apply {
        isEditable = false
        isFocusable = false
        isOpaque = false
        lineWrap = true
        wrapStyleWord = true
        font = WindowsMainFrame.uiFont(Font.PLAIN, 11)
        foreground = Theme.TEXT_SECONDARY
        border = EmptyBorder(0, 0, 0, 0)
        alignmentX = Component.LEFT_ALIGNMENT
    }

    init {
        layout = BoxLayout(this, BoxLayout.Y_AXIS)
        border = EmptyBorder(11, 13, 11, 13)
        cursor = Cursor(Cursor.HAND_CURSOR)
        isFocusable = true

        val topRow = JPanel(BorderLayout()).apply {
            isOpaque = false
            alignmentX = Component.LEFT_ALIGNMENT
            maximumSize = Dimension(Int.MAX_VALUE, 20)
            add(indicatorDot, BorderLayout.WEST)
            add(badgeLabel, BorderLayout.EAST)
        }

        add(topRow)
        add(Box.createVerticalStrut(6))
        add(titleLabel)
        add(Box.createVerticalStrut(4))
        add(descArea)

        val mouseHandler = object : MouseAdapter() {
            override fun mouseEntered(e: MouseEvent?) {
                hovered = true
                updateVisualState()
            }

            override fun mouseExited(e: MouseEvent?) {
                hovered = false
                updateVisualState()
            }

            override fun mousePressed(e: MouseEvent?) {
                requestFocusInWindow()
                onClick()
            }
        }
        addMouseListener(mouseHandler)
        descArea.addMouseListener(mouseHandler)

        addKeyListener(object : KeyAdapter() {
            override fun keyPressed(e: KeyEvent?) {
                if (e?.keyCode == KeyEvent.VK_SPACE || e?.keyCode == KeyEvent.VK_ENTER) {
                    onClick()
                }
            }
        })

        updateVisualState()
    }

    fun setSelectedState(isSelected: Boolean) {
        selected = isSelected
        updateVisualState()
    }

    private fun updateVisualState() {
        fillColor = when {
            selected -> Color(0x16, 0x26, 0x3D)
            hovered -> Color(0x24, 0x31, 0x46)
            else -> Theme.BG_ELEVATED
        }
        borderColor = when {
            selected -> Theme.ACCENT_CYAN
            hovered -> Color(0x47, 0x55, 0x69)
            else -> Theme.BORDER_SUBTLE
        }
        repaint()
    }
}

private class ModernButton(
    text: String,
    private var baseBg: Color,
    private var hoverBg: Color,
    private var borderColor: Color,
    private var textColor: Color,
    private val compact: Boolean = false
) : JButton(text) {

    private var hovered = false
    private var pressed = false

    init {
        isContentAreaFilled = false
        isBorderPainted = false
        isFocusPainted = false
        foreground = textColor
        font = WindowsMainFrame.uiFont(Font.BOLD, if (compact) 11 else 12)
        cursor = Cursor(Cursor.HAND_CURSOR)
        border = if (compact) EmptyBorder(4, 10, 4, 10) else EmptyBorder(9, 16, 9, 16)

        addMouseListener(object : MouseAdapter() {
            override fun mouseEntered(e: MouseEvent?) {
                hovered = true
                repaint()
            }

            override fun mouseExited(e: MouseEvent?) {
                hovered = false
                pressed = false
                repaint()
            }

            override fun mousePressed(e: MouseEvent?) {
                pressed = true
                repaint()
            }

            override fun mouseReleased(e: MouseEvent?) {
                pressed = false
                repaint()
            }
        })
    }

    fun updatePalette(baseBg: Color, hoverBg: Color, borderColor: Color, textColor: Color) {
        this.baseBg = baseBg
        this.hoverBg = hoverBg
        this.borderColor = borderColor
        this.textColor = textColor
        this.foreground = textColor
        repaint()
    }

    override fun paintComponent(g: Graphics) {
        val g2 = g.create() as Graphics2D
        g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)

        val bg = when {
            !isEnabled -> Theme.BG_SURFACE
            pressed -> hoverBg.darker()
            hovered -> hoverBg
            else -> baseBg
        }
        val radius = if (compact) 8f else 10f
        val rect = RoundRectangle2D.Float(0.5f, 0.5f, width - 1f, height - 1f, radius, radius)

        g2.color = bg
        g2.fill(rect)

        g2.color = if (isEnabled) borderColor else Theme.BORDER_SUBTLE
        g2.stroke = BasicStroke(1f)
        g2.draw(rect)

        g2.font = font
        val fm: FontMetrics = g2.fontMetrics
        val textWidth = fm.stringWidth(text ?: "")
        val x = (width - textWidth) / 2
        val y = (height - fm.height) / 2 + fm.ascent
        g2.color = if (isEnabled) textColor else Theme.TEXT_MUTED
        g2.drawString(text ?: "", x, y)

        g2.dispose()
    }
}

private class ModernDarkSliderUI(slider: JSlider) : BasicSliderUI(slider) {
    override fun getThumbSize(): Dimension = Dimension(16, 16)

    override fun paintFocus(g: Graphics) {
        // Focus ring suppressed in favor of clean thumb styling
    }

    override fun paintTrack(g: Graphics) {
        val g2 = g.create() as Graphics2D
        g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)

        val trackHeight = 5f
        val trackY = trackRect.y + (trackRect.height - trackHeight) / 2f
        val fullTrack = RoundRectangle2D.Float(
            trackRect.x.toFloat(),
            trackY,
            trackRect.width.toFloat(),
            trackHeight,
            trackHeight,
            trackHeight
        )
        g2.color = Color(0x33, 0x41, 0x55)
        g2.fill(fullTrack)

        val thumbCenterX = thumbRect.x + thumbRect.width / 2f
        val activeWidth = (thumbCenterX - trackRect.x).coerceAtLeast(0f)
        val activeTrack = RoundRectangle2D.Float(
            trackRect.x.toFloat(),
            trackY,
            activeWidth,
            trackHeight,
            trackHeight,
            trackHeight
        )
        g2.color = Theme.ACCENT_CYAN
        g2.fill(activeTrack)

        g2.dispose()
    }

    override fun paintThumb(g: Graphics) {
        val g2 = g.create() as Graphics2D
        g2.setRenderingHint(RenderingHints.KEY_ANTIALIASING, RenderingHints.VALUE_ANTIALIAS_ON)

        val cx = thumbRect.x.toFloat()
        val cy = thumbRect.y.toFloat()
        val diameter = 15f
        val adjustedY = cy + (thumbRect.height - diameter) / 2f

        g2.color = Theme.TEXT_PRIMARY
        g2.fill(Ellipse2D.Float(cx, adjustedY, diameter, diameter))

        g2.color = Theme.ACCENT_CYAN
        g2.stroke = BasicStroke(2f)
        g2.draw(Ellipse2D.Float(cx, adjustedY, diameter, diameter))

        g2.dispose()
    }
}

fun main() {
    System.setProperty("awt.useSystemAAFontSettings", "on")
    System.setProperty("swing.aatext", "true")

    SwingUtilities.invokeLater {
        try {
            UIManager.setLookAndFeel(UIManager.getCrossPlatformLookAndFeelClassName())
        } catch (_: Exception) {
        }
        val frame = WindowsMainFrame()
        frame.isVisible = true
    }
}
