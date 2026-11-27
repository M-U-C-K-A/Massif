# The DLLs every .exe and .dll of a Windows build of Massif imports, checked
# so that the build starts on a fresh Windows with nothing installed.
#
#   pwsh tools/windows-imports.ps1 <folder of Massif.exe> [-Deploy]
#
# Massif's own files (Massif.exe, massif-cli.exe, MassifThumbnails.dll) are
# built with the static C runtime (.cargo/config.toml) and must import none
# of the Visual C++ runtime's DLLs. A library that comes built (Open Image
# Denoise) may: with -Deploy, the runtime DLLs it needs are copied next to it
# from Visual Studio's redistributable folder (Microsoft's app-local
# deployment: the files of VC\Redist\MSVC\<version>\x64\Microsoft.VC*.CRT are
# Distributable Code under Visual Studio's licence), and checked in turn.
# A Visual C++ runtime DLL needed and not in the folder fails the check: the
# runner has the Redistributable installed, a user's Windows may not.
# Other DLLs neither in the folder nor in System32 are listed as warnings
# (the GPU drivers Open Image Denoise's CUDA and HIP devices open when they
# are there).
param(
  [Parameter(Mandatory = $true)][string]$Folder,
  [switch]$Deploy
)
$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { Write-Error 'windows-imports: no Visual Studio with the C++ tools' }
$dumpbin = Get-ChildItem (Join-Path $vs 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') | Sort-Object FullName | Select-Object -Last 1
if (-not $dumpbin) { Write-Error 'windows-imports: no dumpbin.exe' }

# The Visual C++ runtime's DLLs (and their debug builds, never shipped).
function Is-VcRuntime([string]$name) {
  $name -match '^(vcruntime|msvcp|concrt|vccorlib|vcomp|vcamp|mfc|msvcr)\d.*\.dll$'
}
# Parts of Windows itself: API sets, and the DLLs of System32 that are not
# the Visual C++ runtime (which the runner has installed there).
function Is-System([string]$name) {
  if ($name -match '^(api|ext)-ms-') { return $true }
  if (Is-VcRuntime $name) { return $false }
  Test-Path (Join-Path $env:SystemRoot "System32\$name")
}

# The DLLs a file imports, load-time and delay-loaded, in lower case.
function Imports([string]$file) {
  $out = & $dumpbin.FullName /nologo /dependents $file
  if ($LASTEXITCODE -ne 0) { Write-Error "windows-imports: dumpbin failed on $file" }
  $out | ForEach-Object { $_.Trim() } | Where-Object { $_ -match '\.dll$' -and $_ -notmatch '\s' } | ForEach-Object { $_.ToLowerInvariant() } | Sort-Object -Unique
}

function Redist-Folder {
  $crt = Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT') -Directory | Sort-Object FullName | Select-Object -Last 1
  if (-not $crt) { Write-Error 'windows-imports: no Visual C++ redistributable folder in Visual Studio' }
  $crt.FullName
}

$own = @('massif.exe', 'massif-cli.exe', 'massifthumbnails.dll')
$failed = $false
$checked = @{}
$deployed = @()
do {
  $again = $false
  $files = Get-ChildItem $Folder -File | Where-Object { $_.Extension -in '.exe', '.dll' }
  $present = @{}
  foreach ($f in $files) { $present[$f.Name.ToLowerInvariant()] = $true }
  foreach ($f in $files) {
    $name = $f.Name.ToLowerInvariant()
    if ($checked.ContainsKey($name)) { continue }
    $checked[$name] = $true
    $imports = @(Imports $f.FullName)
    Write-Host "$($f.Name): $($imports -join ', ')"
    foreach ($dll in $imports) {
      if (Is-VcRuntime $dll) {
        if ($own -contains $name) {
          Write-Host "::error::$($f.Name) imports ${dll}: it should be built with the static C runtime (+crt-static)"
          $failed = $true
        } elseif (-not $present.ContainsKey($dll)) {
          $from = if ($Deploy) { Join-Path (Redist-Folder) $dll }
          if ($from -and (Test-Path $from)) {
            Copy-Item $from (Join-Path $Folder $dll)
            $present[$dll] = $true
            $deployed += $dll
            $again = $true
          } else {
            Write-Host "::error::$($f.Name) imports $dll, which is not shipped beside it"
            $failed = $true
          }
        }
      } elseif (-not $present.ContainsKey($dll) -and -not (Is-System $dll)) {
        Write-Host "::warning::$($f.Name) imports $dll, neither shipped nor part of Windows"
      }
    }
  }
} while ($again)

if ($deployed.Count -gt 0) {
  Write-Host "Visual C++ runtime shipped app-local: $(($deployed | Sort-Object -Unique) -join ', ')"
}
if ($failed) { exit 1 }
Write-Host 'windows-imports: every Visual C++ runtime DLL needed is shipped'
