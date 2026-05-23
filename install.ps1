# memora installer for Windows (PowerShell)
# Usage: irm https://raw.githubusercontent.com/harshtripathi272/memora/main/install.ps1 | iex
$ErrorActionPreference = "Stop"

$Repo = "harshtripathi272/memora"
$InstallDir = if ($env:MEMORA_INSTALL_DIR) { $env:MEMORA_INSTALL_DIR } else { "$env:LOCALAPPDATA\memora" }
$Target = "x86_64-pc-windows-msvc"

# Get latest release tag
$Release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
$Tag = $Release.tag_name
if (-not $Tag) { Write-Error "Could not determine latest release."; exit 1 }

$Url = "https://github.com/$Repo/releases/download/$Tag/memora-$Tag-$Target.zip"
$ZipPath = "$env:TEMP\memora-$Tag.zip"

Write-Host "Downloading memora $Tag for Windows..."
Invoke-WebRequest -Uri $Url -OutFile $ZipPath

# Extract
$ExtractDir = "$env:TEMP\memora-extract"
if (Test-Path $ExtractDir) { Remove-Item -Recurse -Force $ExtractDir }
Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir

# Install
if (-not (Test-Path $InstallDir)) { New-Item -ItemType Directory -Path $InstallDir | Out-Null }
Copy-Item "$ExtractDir\memora-$Tag-$Target\memora.exe" "$InstallDir\memora.exe" -Force

# Clean up
Remove-Item $ZipPath -Force
Remove-Item $ExtractDir -Recurse -Force

# Add to PATH if not already there
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path += ";$InstallDir"
    Write-Host "Added $InstallDir to your PATH."
}

Write-Host ""
Write-Host "Installed memora $Tag to $InstallDir\memora.exe" -ForegroundColor Green
Write-Host "Open a new terminal and run: memora --help"
