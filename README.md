# TouchPad Remote — Servidor Windows

Servidor desktop para Windows escrito em **Kotlin (JVM)** que transforma seu celular Android em um touchpad sem fio de baixa latência na mesma rede local (**Wi-Fi / LAN**), com suporte nativo a entrada relativa **Win32 (`user32.dll`)** para jogos em 3D (como **Minecraft**).

> **Aplicativo Android (Celular):** Baixe e acompanhe o aplicativo do celular em **[Henzogabriel954/TouchPadRemote-Android](https://github.com/Henzogabriel954/TouchPadRemote-Android)** (APK disponível em [Releases](https://github.com/Henzogabriel954/TouchPadRemote-Android/releases/latest)).

---

## Downloads (Pronto para Usar)

Baixe a versão mais recente na página oficial de releases:

👉 **[https://github.com/Henzogabriel954/TouchPadRemote-Windows/releases/latest](https://github.com/Henzogabriel954/TouchPadRemote-Windows/releases/latest)**

Arquivos disponíveis em cada release:

1. **`TouchPadRemote-Windows-Portatil.zip`** *(Recomendado)*
   - Versão portátil gerada com `jpackage` contendo **`TouchPadRemote.exe`** e o runtime Java já embutido.
   - **Não precisa instalar Java** no Windows.
   - Extraia a pasta **`TouchPadRemote`** completa (não mova apenas o `.exe` isolado) e execute `TouchPadRemote.exe`.
   - Inclui também o script **`Liberar-Firewall-Windows.bat`** e o guia `LEIA-ME.txt`.

2. **`TouchPadRemote-Servidor.jar`**
   - Executável `.jar` leve (fat JAR) para computadores que já possuem **Java 17 ou superior** instalado.
   - Execute com duplo clique ou via terminal:
     ```powershell
     java -jar TouchPadRemote-Servidor.jar
     ```

---

## Principais Recursos

- **Entrada nativa para jogos 3D (`user32.dll`):** Utiliza `mouse_event` via JNA para enviar deltas relativos puros (`MOUSEEVENTF_MOVE`), cliques e rolagem — essencial para jogos como Minecraft que travam o cursor no centro da tela. Em ambientes sem Win32, usa automaticamente `java.awt.Robot` com *clamping* multi-monitor e acumulação sub-pixel.
- **Baixa latência via UDP (porta `45454`):** Protocolo enxuto para movimento (`MOVE`), cliques (`CLICK`), segurar/soltar botões para arrasto (`BUTTON_STATE`), rolagem (`SCROLL`), descoberta automática na rede (`DISCOVER_REQ` / `DISCOVER_RESP`), medição de latência (`PING` / `PONG`) e pareamento (`PAIR_REQ` / `PAIR_RESP`).
- **Pareamento seguro com aprovação no PC:** O servidor só aceita comandos de mouse após você aprovar o celular na janela do Windows. Ao desconectar um aparelho, eventuais botões pressionados são soltos automaticamente.
- **Detecção inteligente de IP local:** Prioriza interfaces Wi-Fi e Ethernet reais (`192.168.x.x`, `10.x.x.x`), evitando que adaptadores virtuais (WSL, Hyper-V, VMware, VirtualBox, VPNs) atrapalhem a descoberta na rede.
- **Interface escura intuitiva:** Exibe IP local com botão **Copiar IP**, seletor de sensibilidade (`0.5x` a `4.0x`, padrão `1.6x`), controle de iniciar/parar servidor e status em tempo real.

---

## Como Conectar e Parear o Celular

1. **Mesma rede local:** Certifique-se de que o PC Windows e o celular Android estejam conectados ao mesmo roteador (Wi-Fi ou cabo Ethernet no PC + Wi-Fi no celular).
2. **Inicie o servidor no Windows:** Abra o **`TouchPadRemote.exe`** (ou o `.jar`). Confirme que o indicador no topo direito mostra **Ativo na Rede** e que a **Porta UDP** exibida é **`45454`**.
3. **Encontre o computador no aplicativo Android:**
   - Use a busca automática na rede dentro do app **TouchPad Remote**, **ou**
   - Copie o **Endereço IP na Rede Local** mostrado na janela do Windows e cadastre manualmente junto com a porta **`45454`**.
4. **Solicite o pareamento no celular:** Toque na área do touchpad no celular para iniciar a conexão. A janela do Windows virá para frente automaticamente exibindo o card **Pareamento do Celular** com o nome e o IP do aparelho.
5. **Autorize no Windows:** Clique em **`Aceitar Pareamento`** (ou **`Recusar`** para bloquear). Assim que aprovado, você já pode mover o cursor, clicar, arrastar e rolar.
6. **Para encerrar o acesso:** Clique em **`Desconectar`** ao lado do dispositivo autorizado ou em **`Parar Servidor`**.

---

## Configuração do Firewall do Windows (Porta UDP `45454`)

Se o aplicativo do celular exibir o computador como **Offline** ou não encontrar o PC na busca automática mesmo com o servidor aberto na mesma rede:

1. **Aviso inicial do Windows:** Ao abrir o `TouchPadRemote.exe` pela primeira vez, clique em **"Permitir acesso"** (recomendado marcar redes privadas e públicas se sua rede Wi-Fi estiver configurada como pública).
2. **Script automático incluso (`Liberar-Firewall-Windows.bat`):**
   - Na pasta extraída do `TouchPadRemote-Windows-Portatil.zip`, execute **`Liberar-Firewall-Windows.bat`** (ele solicitará permissão de **Administrador**).
   - O script cria regras de entrada no Firewall do Windows liberando o executável `TouchPadRemote.exe` e a porta **UDP `45454`**.
3. **Liberação manual (alternativa):**
   - Abra o **Windows Defender Firewall com Segurança Avançada** → **Regras de Entrada** → **Nova Regra...** → **Porta** → **UDP** → **Portas locais específicas: `45454`** → **Permitir a conexão**.

---

## Compilando a Partir do Código-Fonte

### Pré-requisitos
- **JDK 22** (necessário para gerar o `.exe` portátil via `jpackage` na tarefa `packagePortableWindows`; para apenas compilar/executar o `.jar` e os testes, **JDK 17+** já é suficiente).
- **Gradle 8.13** instalado no `PATH` (ou utilize o wrapper local caso esteja em um workspace com `gradlew.bat`).

### Comandos

Executar todos os testes (`shared-protocol` e `windows-app`):

```powershell
gradle :shared-protocol:test :windows-app:test
```

Compilar o JAR executável (`windows-app/build/libs/windows-app.jar`):

```powershell
gradle :windows-app:jar
```

Compilar (se necessário) e iniciar o servidor diretamente no Windows:

```bat
run-windows-server.bat
```

Gerar a pasta portátil (`dist/TouchPadRemote/`) e o arquivo ZIP (`dist/TouchPadRemote-Windows-Portatil.zip`):

```powershell
gradle :windows-app:packagePortableWindows
```

---

## Estrutura do Repositório

```text
├── shared-protocol/
│   ├── build.gradle.kts
│   └── src/
│       ├── main/kotlin/com/touchpad/protocol/
│       │   ├── ConnectionMethod.kt        # Métodos de conexão (NETWORK_LAN, BLUETOOTH, USB_ADB)
│       │   └── TouchpadPacket.kt          # Codec e pacotes do protocolo UDP (porta 45454)
│       └── test/kotlin/com/touchpad/protocol/
│           └── TouchpadPacketTest.kt      # Testes unitários do protocolo
├── windows-app/
│   ├── build.gradle.kts                   # Build do fat JAR, jpackage app-image e ZIP portátil
│   └── src/
│       ├── main/kotlin/com/touchpad/windows/
│       │   ├── WindowsMouseController.kt  # Controle Win32 user32.dll + fallback Robot + clamping
│       │   ├── NetworkTouchpadServer.kt   # Servidor UDP, descoberta, ping e fluxo de pareamento
│       │   └── WindowsMainFrame.kt        # Interface gráfica Swing (tema escuro)
│       └── test/kotlin/com/touchpad/windows/
│           └── NetworkServerTest.kt       # Testes de servidor UDP, pareamento e controle de mouse
├── .github/workflows/release.yml          # Workflow GitHub Actions (testes, JAR, jpackage e Release)
├── gradle/libs.versions.toml              # Catálogo de versões (Kotlin 2.0.21)
├── build.gradle.kts
├── settings.gradle.kts
├── gradle.properties
└── run-windows-server.bat                 # Atalho para compilar e executar o servidor JAR no Windows
```
