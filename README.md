# TouchPad Remote — Servidor Windows (Rust)

Servidor desktop e interface gráfica nativa em **Rust** (`egui` + `windows-sys`) que transforma um celular Android em touchpad sem fio de baixa latência via UDP (`45454`), com suporte a jogos 3D, telas administrativas/UAC e troca Multi-PC pela borda.

## Recursos (v2.0.0)
- **Executável nativo (~6,4 MB)** sem necessidade de instalar Java.
- **Jogos 3D (Minecraft, FPS, etc.)**: movimento relativo puro via Win32 `SendInput` (`MOUSEEVENTF_MOVE`) com trava automática contra troca acidental de PC quando o cursor está preso/oculto.
- **Gerenciador de Tarefas e Janelas UAC ("Sim / Não")**: botão de 1 clique **"Instalar Serviço UAC"** (`touchpad-win-service.exe` rodando como `LocalSystem` com agente no desktop seguro `Winlogon` e pipes restritos por SDDL), além de opção **"Reiniciar como Administrador"** no modo portátil.
- **Persistência atômica**: configurações (`sensibilidade`, `porta`, `resistência de borda`, `bandeja`) e celulares permitidos salvos automaticamente (`%APPDATA%\TouchPadRemote\config.json` ou `C:\ProgramData\TouchPadRemote\config.json`).
- **Multi-PC por bordas**: o celular conecta nos PCs desejados e envia `EDGE_CONFIG` / recebe `EDGE_HIT` sem nenhuma conexão direta entre os computadores.

## Build local
```powershell
cargo test --workspace
cargo build --release -p touchpad-desktop-gui -p touchpad-win-service
./packaging/windows/package-windows.ps1 -SkipBuild
```

## Repositórios Relacionados
- **App Android**: https://github.com/Henzogabriel954/TouchPadRemote-Android
- **Servidor Linux**: https://github.com/Henzogabriel954/TouchPadRemote-Linux
