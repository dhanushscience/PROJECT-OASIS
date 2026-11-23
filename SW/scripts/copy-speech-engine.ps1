# Copies the speech-engine files OASIS needs for push-to-talk into a `native` folder beside the
# program: sherpa-onnx **2.0.2** (the C structs in oasis-voice/src/sherpa.rs mirror its c-api.h; any
# other version reads garbage). OASIS never downloads them. Run once after building:
#
#   .\scripts\copy-speech-engine.ps1 -From "C:\path\to\sherpa-onnx-v2.0.2-win-x64-shared-MT-Release-no-tts\lib"
#
# -From is the `lib` folder of the GitHub release asset sherpa-onnx-v2.0.2-win-x64-shared-MT-Release-no-tts
# (k2-fsa/sherpa-onnx): sherpa-onnx-c-api.dll, onnxruntime.dll and onnxruntime_providers_shared.dll.
param(
    [Parameter(Mandatory = $true)][string]$From,
    [string]$To = (Join-Path $PSScriptRoot "..\target\debug\native")
)

$files = "sherpa-onnx-c-api.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"
$source = @($From, (Join-Path $From "runtimes\win-x64\native")) |
    Where-Object { Test-Path (Join-Path $_ $files[0]) } | Select-Object -First 1
if (-not $source) { throw "Could not find $($files[0]) under $From." }

New-Item -ItemType Directory -Force $To | Out-Null
foreach ($f in $files) {
    Copy-Item (Join-Path $source $f) (Join-Path $To $f) -Force
    "copied $f"
}
"Done. Speech engine files are in $To"
