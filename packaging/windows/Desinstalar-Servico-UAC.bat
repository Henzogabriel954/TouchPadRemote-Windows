@echo off
setlocal
set "TOUCHPAD_SERVICE_EXE=%~dp0touchpad-win-service.exe"
if not exist "%TOUCHPAD_SERVICE_EXE%" (
    echo touchpad-win-service.exe nao encontrado. Extraia todos os arquivos do ZIP.
    pause
    exit /b 1
)
echo Solicitando permissao de Administrador para desinstalar o Servico UAC...
powershell.exe -NoProfile -Command "try { $process = Start-Process -FilePath $env:TOUCHPAD_SERVICE_EXE -WorkingDirectory (Split-Path -LiteralPath $env:TOUCHPAD_SERVICE_EXE) -ArgumentList '--uninstall' -Verb RunAs -Wait -PassThru -ErrorAction Stop; exit $process.ExitCode } catch { Write-Host $_; exit 1 }"
set "resultado=%errorlevel%"
if not "%resultado%"=="0" (
    echo Nao foi possivel desinstalar o Servico UAC.
    pause
)
exit /b %resultado%
