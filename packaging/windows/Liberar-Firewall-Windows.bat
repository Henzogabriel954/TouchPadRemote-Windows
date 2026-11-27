@echo off
setlocal
set "TOUCHPAD_PORTABLE_DIR=%~dp0"
if not exist "%~dp0TouchPadRemote.exe" (
    echo TouchPadRemote.exe nao encontrado. Extraia todos os arquivos do ZIP.
    pause
    exit /b 1
)
if not exist "%~dp0touchpad-win-service.exe" (
    echo touchpad-win-service.exe nao encontrado. Extraia todos os arquivos do ZIP.
    pause
    exit /b 1
)
echo Solicitando permissao de Administrador para liberar o TouchPad Remote no Firewall do Windows ^(UDP 45454^)...
powershell.exe -NoProfile -Command "try { $dir = $env:TOUCHPAD_PORTABLE_DIR.Replace([string][char]39, ([string][char]39) * 2); $rules = { param($dir) $ErrorActionPreference = 'Stop'; try { foreach ($exe in 'TouchPadRemote.exe', 'touchpad-win-service.exe') { $name = 'TouchPadRemote-UDP-' + $exe; $program = Join-Path $dir $exe; $rule = Get-NetFirewallRule -Name $name -ErrorAction SilentlyContinue; if ($rule) { $rule | Remove-NetFirewallRule }; New-NetFirewallRule -Name $name -DisplayName ('TouchPad Remote UDP 45454 - ' + $exe) -Direction Inbound -Action Allow -Program $program -Protocol UDP -LocalPort 45454 | Out-Null }; exit 0 } catch { Write-Host $_; exit 1 } }; $command = '& {' + $rules.ToString() + '} ' + [char]39 + $dir + [char]39; $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command)); $process = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -ArgumentList '-NoProfile', '-EncodedCommand', $encoded -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $process.ExitCode } catch { Write-Host $_; exit 1 }"
set "resultado=%errorlevel%"
if "%resultado%"=="0" (
    echo Firewall liberado para o aplicativo e o servico na porta UDP 45454.
) else (
    echo Nao foi possivel liberar o Firewall do Windows.
)
pause
exit /b %resultado%
