[CmdletBinding()]
param(
    [string]$CsdkRoot = $env:DEADLOCK_CSDK,
    [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"

if (Get-Variable -Name PSNativeCommandUseErrorActionPreference -ErrorAction SilentlyContinue) {
    $PSNativeCommandUseErrorActionPreference = $false
}

$ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$SourceRoot = Join-Path $ProjectRoot "mod"
$BuildName = "death_http_bridge"

if ([string]::IsNullOrWhiteSpace($CsdkRoot)) {
    $CandidateCsdkRoots = @(
        "C:\Reduced_CSDK_12",
        "C:\deadlock_builds\Reduced_CSDK_12",
        (Join-Path $env:USERPROFILE "Desktop\Reduced_CSDK_12")
    )
    $CsdkRoot = $CandidateCsdkRoots | Where-Object {
        Test-Path (Join-Path $_ "game\bin_cs2\win64\resourcecompiler.exe") -PathType Leaf
    } | Select-Object -First 1
    if (-not $CsdkRoot) {
        $CsdkRoot = $CandidateCsdkRoots[0]
    }
}

if ([string]::IsNullOrWhiteSpace($OutputPath)) {
    $OutputPath = Join-Path $ProjectRoot "dist\deadlock_death_hook.vpk"
} elseif (-not [System.IO.Path]::IsPathRooted($OutputPath)) {
    $OutputPath = Join-Path $ProjectRoot $OutputPath
}

$Compiler = Join-Path $CsdkRoot "game\bin_cs2\win64\resourcecompiler.exe"
$Packer = Join-Path $CsdkRoot "game\bin\win64\CSDKCfgVPK.exe"
$ContentBuildRoot = Join-Path $CsdkRoot "content\citadel_addons\$BuildName"
$GameBuildRoot = Join-Path $CsdkRoot "game\citadel_addons\$BuildName"

if (-not (Test-Path $Compiler -PathType Leaf)) {
    throw "Reduced CSDK resource compiler not found: $Compiler"
}

if (-not (Test-Path $Packer -PathType Leaf)) {
    throw "Reduced CSDK VPK packer not found: $Packer"
}

if (-not (Test-Path $SourceRoot -PathType Container)) {
    throw "Mod source folder not found: $SourceRoot"
}

Remove-Item $ContentBuildRoot -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $GameBuildRoot -Recurse -Force -ErrorAction SilentlyContinue
New-Item $ContentBuildRoot -ItemType Directory -Force | Out-Null
New-Item $GameBuildRoot -ItemType Directory -Force | Out-Null

Copy-Item (Join-Path $SourceRoot "*") $ContentBuildRoot -Recurse -Force

$Sources = Get-ChildItem $ContentBuildRoot -Recurse -File | Where-Object {
    $_.Extension -in @(".js", ".xml")
}

if ($Sources.Count -eq 0) {
    throw "No Panorama sources found under $SourceRoot"
}

foreach ($Source in $Sources) {
    Write-Host "Compiling $($Source.FullName.Substring($ContentBuildRoot.Length + 1))"
    & $Compiler "-i" $Source.FullName "-nop4"
    if ($LASTEXITCODE -ne 0) {
        throw "Resource compiler failed for $($Source.FullName) with exit code $LASTEXITCODE"
    }
}

$OutputDirectory = Split-Path -Parent $OutputPath
New-Item $OutputDirectory -ItemType Directory -Force | Out-Null
Remove-Item $OutputPath -Force -ErrorAction SilentlyContinue

& $Packer $GameBuildRoot $OutputPath
if ($LASTEXITCODE -ne 0) {
    throw "VPK packer failed with exit code $LASTEXITCODE"
}

Write-Host "Built $OutputPath"
