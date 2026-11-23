# Builds the beta installer: the OASIS app with the speech engine and the
# browser extension inside it (voice models download on first start). Testers bring their own Groq key; none is packed.
#
#   .\scripts\package-beta.ps1
#
# -SpeechEngine: the folder with sherpa-onnx-c-api.dll and onnxruntime.dll (see copy-speech-engine.ps1).
# -Notes: what changed, shown to testers in Settings > Updates.
# -Publish: also make the GitHub release that installed copies update from (see docs/UPDATES.md).
# The installer, its signature and latest.json are written to target\release\bundle\nsis\.
param(
    [string]$SpeechEngine = (Join-Path $PSScriptRoot "..\target\debug\native"),
    [string]$Notes = "",
    [switch]$Publish,
    # The public repo installed copies read latest.json from; must match the endpoint in tauri.conf.json.
    [string]$ReleaseRepo = "dhanushscience/PROJECT-OASIS",
    [string]$SigningKey = (Join-Path $env:USERPROFILE ".tauri\oasis-updater.key")
)
$ErrorActionPreference = "Stop"
$sw = Resolve-Path (Join-Path $PSScriptRoot "..")
$stage = Join-Path $sw "dist\stage"

# 1. Stage what the installer carries.
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
$native = New-Item -ItemType Directory -Force (Join-Path $stage "native")
foreach ($f in "sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll") {
    $src = Join-Path $SpeechEngine $f
    if (-not (Test-Path $src)) { throw "Missing $src. Run copy-speech-engine.ps1 first, or pass -SpeechEngine." }
    Copy-Item $src $native
}
# No speech model is packed: the app downloads the set that fits the PC on first start.

# 2. The window's page, then the installer (the Tauri CLI matches the tauri crate, 2.12).
Push-Location (Join-Path $sw "apps\oasis-desktop\web")
try { npm run build; if ($LASTEXITCODE) { throw "The page build failed." } } finally { Pop-Location }
# The updater only accepts installers signed with this key (its public half is in tauri.conf.json).
if (-not (Test-Path $SigningKey)) { throw "No updater signing key at $SigningKey. See docs/UPDATES.md." }
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $SigningKey -Raw
if (-not $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD) { $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = "" }
Push-Location (Join-Path $sw "apps\oasis-desktop")
try {
    npx --yes @tauri-apps/cli@2.12.4 build --config tauri.beta.conf.json
    if ($LASTEXITCODE) { throw "The installer build failed." }
} finally { Pop-Location }

$out = Get-ChildItem (Join-Path $sw "target\release\bundle\nsis\*.exe") | Sort-Object LastWriteTime | Select-Object -Last 1
"Installer: $($out.FullName) ($([math]::Round($out.Length / 1MB)) MB)"

# 3. latest.json: what installed copies read to find this version.
$version = (Get-Content (Join-Path $sw "apps\oasis-desktop\tauri.conf.json") -Raw | ConvertFrom-Json).version
$tag = "v$version"
$sig = "$($out.FullName).sig"
if (-not (Test-Path $sig)) { throw "The installer was not signed ($sig is missing)." }
$manifest = [ordered]@{
    version   = $version
    notes     = $Notes
    pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    platforms = @{ "windows-x86_64" = @{
        signature = (Get-Content $sig -Raw).Trim()
        url       = "https://github.com/$ReleaseRepo/releases/download/$tag/$($out.Name)"
    } }
}
$latest = Join-Path $out.DirectoryName "latest.json"
# no BOM: the updater parses this as plain JSON
[IO.File]::WriteAllText($latest, ($manifest | ConvertTo-Json -Depth 4))
"Update manifest: $latest"

# 4. The release. It is marked latest, so /releases/latest/download/latest.json now points here.
if ($Publish) {
    gh release create $tag $out.FullName $latest --repo $ReleaseRepo --title "OASIS $version" --notes ($(if ($Notes) { $Notes } else { "OASIS $version" })) --latest
    if ($LASTEXITCODE) { throw "The release was not published." }
    # A copy with a fixed name, so the README's download button (.../releases/latest/download/OASIS-setup.exe) never goes stale.
    $alias = Join-Path $out.DirectoryName "OASIS-setup.exe"
    Copy-Item $out.FullName $alias -Force
    gh release upload $tag $alias --repo $ReleaseRepo --clobber
    if ($LASTEXITCODE) { throw "The fixed-name download copy was not uploaded." }
    "Published $tag to $ReleaseRepo."
}
