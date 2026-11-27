[CmdletBinding()]
param([switch]$SkipBuild)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\..')).Path
$dist = Join-Path $repoRoot 'dist'
$portable = Join-Path $dist 'TouchPadRemote'
$archive = Join-Path $dist 'TouchPadRemote-Windows-Portatil.zip'

if (-not $SkipBuild) {
    Push-Location -LiteralPath $repoRoot
    try {
        Write-Host 'Compilando a versao portatil do TouchPad Remote...'
        cargo build --release -p touchpad-desktop-gui -p touchpad-win-service
        if ($LASTEXITCODE -ne 0) {
            throw "A compilacao falhou (codigo $LASTEXITCODE)."
        }
    } finally {
        Pop-Location
    }
}

$release = Join-Path $repoRoot 'target\release'
$files = @(
    (Join-Path $release 'TouchPadRemote.exe')
    (Join-Path $release 'touchpad-win-service.exe')
    (Join-Path $PSScriptRoot 'Liberar-Firewall-Windows.bat')
    (Join-Path $PSScriptRoot 'Instalar-Servico-UAC.bat')
    (Join-Path $PSScriptRoot 'Desinstalar-Servico-UAC.bat')
    (Join-Path $PSScriptRoot 'LEIA-ME.txt')
)
foreach ($file in $files) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "Arquivo necessario para o pacote: $file"
    }
}

$staging = New-Item -ItemType Directory -Path (Join-Path $dist ([guid]::NewGuid().ToString()))
try {
    $stagedPortable = New-Item -ItemType Directory -Path (Join-Path $staging.FullName 'TouchPadRemote')
    Copy-Item -LiteralPath $files -Destination $stagedPortable.FullName -Force
    New-Item -ItemType Directory -Path $portable -Force | Out-Null
    Copy-Item -LiteralPath $files -Destination $portable -Force
    Compress-Archive -LiteralPath $stagedPortable.FullName -DestinationPath $archive -Force
    Write-Host "Pacote pronto: $archive"
} finally {
    Get-ChildItem -LiteralPath (Join-Path $staging.FullName 'TouchPadRemote') -File -ErrorAction SilentlyContinue |
        Remove-Item -Force
    if (Test-Path -LiteralPath (Join-Path $staging.FullName 'TouchPadRemote')) {
        Remove-Item -LiteralPath (Join-Path $staging.FullName 'TouchPadRemote')
    }
    Remove-Item -LiteralPath $staging.FullName
}
