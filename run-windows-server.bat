@echo off
setlocal

set "JAR_PATH=%~dp0windows-app\build\libs\windows-app.jar"
set "JDK22_JAVA=C:\Program Files\Java\jdk-22\bin\java.exe"
set "JBR_JAVA=C:\Program Files\Android\Android Studio\jbr\bin\java.exe"

if not exist "%JAR_PATH%" (
    echo [TouchPad Remote] JAR nao encontrado. Compilando :windows-app:jar...
    if exist "%~dp0gradlew.bat" (
        call "%~dp0gradlew.bat" :windows-app:jar
    ) else (
        call gradle :windows-app:jar
    )
    if errorlevel 1 (
        echo [TouchPad Remote] Erro ao compilar o servidor Windows.
        pause
        exit /b 1
    )
)

if exist "%JDK22_JAVA%" (
    set "JAVA_EXE=%JDK22_JAVA%"
) else if exist "%JBR_JAVA%" (
    set "JAVA_EXE=%JBR_JAVA%"
) else (
    set "JAVA_EXE=java"
)

echo [TouchPad Remote] Usando Java: "%JAVA_EXE%"
echo [TouchPad Remote] Iniciando TouchPad Remote - Servidor Windows...
"%JAVA_EXE%" -jar "%JAR_PATH%"
if errorlevel 1 (
    echo [TouchPad Remote] O servidor encerrou com erro.
    pause
    exit /b 1
)

endlocal
exit /b 0
