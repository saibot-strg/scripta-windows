# Builds sherpa-onnx with ONNX Runtime's DirectML execution provider, so
# speech recognition and speaker separation can run on the graphics card
# (AMD, NVIDIA and Intel alike). The prebuilt sherpa-onnx libraries for
# Windows are CPU-only, or CUDA (NVIDIA only).
#
# sherpa-onnx itself is built as static libraries and linked into scripta.exe:
# Smart App Control blocks unsigned DLLs without reputation (it did block a
# self-built sherpa-onnx-c-api.dll). Only Microsoft-signed DLLs ship next to
# the app: onnxruntime, DirectML and the Visual C++ runtime.
#
# Output: src-tauri\native\sherpa-onnx. src-tauri\.cargo\config.toml points
# SHERPA_ONNX_LIB_DIR there and tauri.conf.json ships the DLLs.
#
# Needs git and Visual Studio 2022 with the C++ workload (it brings CMake).
#   powershell -ExecutionPolicy Bypass -File scripts\build-sherpa-directml.ps1

param(
    # Must match the version of the sherpa-onnx crate in src-tauri\Cargo.toml.
    [string]$SherpaVersion = '1.13.8',
    [string]$OrtVersion = '1.24.4',
    [string]$DirectMLVersion = '1.15.4',
    [string]$WorkDir = (Join-Path $env:LOCALAPPDATA 'scripta-build'),
    [string]$OutDir = (Join-Path $PSScriptRoot '..\src-tauri\native\sherpa-onnx')
)

$ErrorActionPreference = 'Stop'
# Invoke-WebRequest is very slow while it draws a progress bar.
$ProgressPreference = 'SilentlyContinue'

$stamp = "sherpa-onnx $SherpaVersion (static, adapter by LUID, decoder patch), onnxruntime-directml $OrtVersion, DirectML $DirectMLVersion, VC runtime"
$stampFile = Join-Path $OutDir 'VERSION.txt'
if ((Test-Path $stampFile) -and ((Get-Content $stampFile -Raw).Trim() -eq $stamp)) {
    Write-Host "Up to date: $stamp"
    exit 0
}

# Visual Studio with the C++ workload: its CMake, generator and C++ runtime.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vsArgs = @('-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64')
$vs = & $vswhere @vsArgs -property installationPath
if (-not $vs) { throw 'Visual Studio with the C++ workload not found.' }
$generator = switch (([version](& $vswhere @vsArgs -property installationVersion)).Major) {
    17 { 'Visual Studio 17 2022' }
    18 { 'Visual Studio 18 2026' }
    default { throw "Unsupported Visual Studio version in $vs" }
}
$cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
if (-not $cmake) { $cmake = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe' }

# NuGet packages are zip files; the .zip name lets CMake extract them.
function Get-Package([string]$id, [string]$version) {
    $name = $id.ToLower()
    $file = Join-Path $downloads "$name.$version.zip"
    if (-not (Test-Path $file)) {
        Write-Host "Downloading $id $version"
        Invoke-WebRequest "https://api.nuget.org/v3-flatcontainer/$name/$version/$name.$version.nupkg" -OutFile "$file.part"
        Move-Item "$file.part" $file
    }
    return $file
}

$downloads = Join-Path $WorkDir 'downloads'
New-Item -ItemType Directory -Force $downloads | Out-Null
$ort = Get-Package 'Microsoft.ML.OnnxRuntime.DirectML' $OrtVersion
$directml = Get-Package 'Microsoft.AI.DirectML' $DirectMLVersion

$source = Join-Path $WorkDir "sherpa-onnx-$SherpaVersion"
if (-not (Test-Path (Join-Path $source 'CMakeLists.txt'))) {
    git clone --quiet --depth 1 --branch "v$SherpaVersion" https://github.com/k2-fsa/sherpa-onnx.git $source
    if ($LASTEXITCODE) { throw 'git clone failed' }
}

# The patches below start from the released sources every time.
git -C $source checkout --quiet -- cmake/onnxruntime-win-x64-directml.cmake CMakeLists.txt sherpa-onnx/csrc/session.cc sherpa-onnx/csrc/offline-transducer-nemo-model.cc
if ($LASTEXITCODE) { throw 'git checkout failed' }

# sherpa-onnx pins onnxruntime 1.14.1 and DirectML 1.15.0 for DirectML builds;
# point it at the newer packages downloaded above.
$cmakeFile = Join-Path $source 'cmake\onnxruntime-win-x64-directml.cmake'
$text = Get-Content $cmakeFile -Raw
$text = $text -replace 'set\(onnxruntime_URL\s+"[^"]*"\)', ('set(onnxruntime_URL "{0}")' -f ($ort -replace '\\', '/'))
$text = $text -replace 'set\(onnxruntime_HASH\s+"[^"]*"\)', ('set(onnxruntime_HASH "SHA256={0}")' -f (Get-FileHash $ort -Algorithm SHA256).Hash)
$text = $text -replace 'set\(directml_URL\s+"[^"]*"\)', ('set(directml_URL "{0}")' -f ($directml -replace '\\', '/'))
$text = $text -replace 'set\(directml_HASH\s+"[^"]*"\)', ('set(directml_HASH "SHA256={0}")' -f (Get-FileHash $directml -Algorithm SHA256).Hash)
# DirectML builds are shared-only upstream; static sherpa-onnx libraries
# linked against the onnxruntime DLL work just as well.
$text = $text.Replace('message(FATAL_ERROR "This file is for building shared libraries.', 'message(STATUS "Static sherpa-onnx, shared onnxruntime.')
Set-Content -Path $cmakeFile -Value $text -NoNewline
$listsFile = Join-Path $source 'CMakeLists.txt'
$lists = Get-Content $listsFile -Raw
$lists = $lists -replace '(since SHERPA_ONNX_ENABLE_DIRECTML is ON"\)\s*)set\(BUILD_SHARED_LIBS ON CACHE BOOL "" FORCE\)', '$1'
Set-Content -Path $listsFile -Value $lists -NoNewline

# sherpa-onnx always opens graphics adapter 0. The app names the card by its
# LUID instead (SHERPA_ONNX_DML_ADAPTER_LUID, "high:low" in hex): adapter
# numbers differ between sessions and APIs (over SSH the RX 7900 XTX was
# DXGI adapter 1, on the desktop 0, and onnxruntime's 0 was then the
# integrated graphics).
$helper = @'
#include <cstdio>
#include <cstdlib>
#include <dxgi1_4.h>
#include <wrl/client.h>
#pragma comment(lib, "d3d12.lib")
#pragma comment(lib, "dxgi.lib")
#pragma comment(lib, "DirectML.lib")
static OrtStatus *ScriptaAppendDml(OrtSessionOptions *options) {
  const char *text = std::getenv("SHERPA_ONNX_DML_ADAPTER_LUID");
  unsigned long high = 0, low = 0;
  if (!text || std::sscanf(text, "%lx:%lx", &high, &low) != 2) {
    return OrtSessionOptionsAppendExecutionProvider_DML(options, 0);
  }
  LUID luid;
  luid.HighPart = static_cast<LONG>(high);
  luid.LowPart = static_cast<DWORD>(low);
  Microsoft::WRL::ComPtr<IDXGIFactory4> factory;
  Microsoft::WRL::ComPtr<IDXGIAdapter1> adapter;
  Microsoft::WRL::ComPtr<ID3D12Device> device;
  Microsoft::WRL::ComPtr<ID3D12CommandQueue> queue;
  Microsoft::WRL::ComPtr<IDMLDevice> dml;
  D3D12_COMMAND_QUEUE_DESC desc = {};
  desc.Type = D3D12_COMMAND_LIST_TYPE_DIRECT;
  desc.Flags = D3D12_COMMAND_QUEUE_FLAG_DISABLE_GPU_TIMEOUT;
  const OrtApi &ort = Ort::GetApi();
  if (FAILED(CreateDXGIFactory2(0, IID_PPV_ARGS(&factory))) ||
      FAILED(factory->EnumAdapterByLuid(luid, IID_PPV_ARGS(&adapter))) ||
      FAILED(D3D12CreateDevice(adapter.Get(), D3D_FEATURE_LEVEL_11_0, IID_PPV_ARGS(&device))) ||
      FAILED(device->CreateCommandQueue(&desc, IID_PPV_ARGS(&queue))) ||
      FAILED(DMLCreateDevice(device.Get(), DML_CREATE_DEVICE_FLAG_NONE, IID_PPV_ARGS(&dml)))) {
    return ort.CreateStatus(ORT_FAIL, "the graphics card could not be opened");
  }
  const OrtDmlApi *dml_api = nullptr;
  OrtStatus *status = ort.GetExecutionProviderApi("DML", ORT_API_VERSION, reinterpret_cast<const void **>(&dml_api));
  if (status) {
    return status;
  }
  return dml_api->SessionOptionsAppendExecutionProvider_DML1(options, dml.Get(), queue.Get());
}
'@
$sessionFile = Join-Path $source 'sherpa-onnx\csrc\session.cc'
$session = Get-Content $sessionFile -Raw
$session = $session.Replace('#include "dml_provider_factory.h"  // NOLINT', "#include `"dml_provider_factory.h`"  // NOLINT`n$helper")
$session = $session -replace 'int32_t device_id = 0;\s*OrtStatus \*status =\s*OrtSessionOptionsAppendExecutionProvider_DML\(sess_opts, device_id\);', 'OrtStatus *status = ScriptaAppendDml(sess_opts);'
if (([regex]::Matches($session, 'ScriptaAppendDml')).Count -ne 2) { throw "session.cc changed: the DirectML adapter patch no longer applies" }
Set-Content -Path $sessionFile -Value $session -NoNewline

# Parakeet's decoder and joiner are tiny but run once per audio frame; only
# the encoder goes to the graphics card.
$nemoFile = Join-Path $source 'sherpa-onnx\csrc\offline-transducer-nemo-model.cc'
$nemo = Get-Content $nemoFile -Raw
$nemo = $nemo -replace 'sess_opts_\(GetSessionOptions\(config\)\),', 'sess_opts_(GetSessionOptions(config)), cpu_sess_opts_(GetSessionOptionsImpl(config.num_threads, "cpu")),'
$nemo = $nemo -replace '(config\.transducer\.(decoder|joiner)_filename\),\s+)sess_opts_\)', '$1cpu_sess_opts_)'
$nemo = $nemo -replace 'Ort::SessionOptions sess_opts_;', 'Ort::SessionOptions sess_opts_; Ort::SessionOptions cpu_sess_opts_;'
if (([regex]::Matches($nemo, 'cpu_sess_opts_')).Count -ne 5) { throw "offline-transducer-nemo-model.cc changed: the decoder patch no longer applies" }
Set-Content -Path $nemoFile -Value $nemo -NoNewline

$build = Join-Path $WorkDir "build-static-$SherpaVersion"
$install = Join-Path $WorkDir "install-static-$SherpaVersion"
# The static C runtime (/MT), like the app. TTS stays on: the Rust crate
# links its libraries too.
& $cmake -S $source -B $build -G $generator -A x64 `
    -DCMAKE_BUILD_TYPE=Release `
    -DCMAKE_INSTALL_PREFIX="$install" `
    -DBUILD_SHARED_LIBS=OFF `
    -DSHERPA_ONNX_ENABLE_DIRECTML=ON `
    -DSHERPA_ONNX_USE_STATIC_CRT=ON `
    -DSHERPA_ONNX_ENABLE_C_API=ON `
    -DSHERPA_ONNX_ENABLE_BINARY=OFF `
    -DSHERPA_ONNX_BUILD_C_API_EXAMPLES=OFF `
    -DSHERPA_ONNX_ENABLE_PORTAUDIO=OFF `
    -DSHERPA_ONNX_ENABLE_WEBSOCKET=OFF `
    -DSHERPA_ONNX_ENABLE_PYTHON=OFF `
    -DSHERPA_ONNX_ENABLE_TESTS=OFF
if ($LASTEXITCODE) { throw 'CMake configure failed' }
& $cmake --build $build --config Release --parallel
if ($LASTEXITCODE) { throw 'CMake build failed' }
& $cmake --install $build --config Release
if ($LASTEXITCODE) { throw 'CMake install failed' }

if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
New-Item -ItemType Directory -Force $OutDir | Out-Null
# The static libraries the sherpa-onnx crate links, plus the import library
# of onnxruntime.dll.
Copy-Item (Join-Path $install 'lib\*.lib') $OutDir
if (-not (Test-Path (Join-Path $OutDir 'sherpa-onnx-core.lib'))) { throw "no static sherpa-onnx libraries in $install\lib" }
foreach ($name in 'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'DirectML.dll') {
    $file = Get-ChildItem $install -Recurse -Filter $name | Select-Object -First 1
    if ($file) { Copy-Item $file.FullName $OutDir }
    elseif ($name -ne 'onnxruntime_providers_shared.dll') { throw "$name missing in $install" }
}

# onnxruntime.dll needs the Visual C++ runtime, which office PCs don't always
# have; it is shipped next to the app (app-local deployment). It must be at
# least as new as the toolset onnxruntime was built with (14.44 for 1.24.4).
$crt = Get-ChildItem (Join-Path $vs 'VC\Redist\MSVC') -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Get-ChildItem (Join-Path $_.FullName 'x64') -Directory -Filter 'Microsoft.VC*.CRT' } |
    Select-Object -First 1
if (-not $crt) { throw 'Visual C++ runtime (VC\Redist) not found in Visual Studio' }
foreach ($name in 'msvcp140.dll', 'msvcp140_1.dll', 'vcruntime140.dll', 'vcruntime140_1.dll') {
    Copy-Item (Join-Path $crt.FullName $name) $OutDir
}
Write-Host "Visual C++ runtime from $($crt.FullName)"

Set-Content -Path $stampFile -Value $stamp

# The sherpa-onnx-sys crate copies the static libraries into its compiled
# rlib and only rebuilds when SHERPA_ONNX_LIB_DIR changes: without this the
# app kept linking the first libraries built here.
$manifest = Join-Path $PSScriptRoot '..\src-tauri\Cargo.toml'
if (Get-Command cargo -ErrorAction SilentlyContinue) {
    & cargo clean --quiet --manifest-path $manifest --package sherpa-onnx-sys --target x86_64-pc-windows-msvc --release
    & cargo clean --quiet --manifest-path $manifest --package sherpa-onnx-sys --target x86_64-pc-windows-msvc
}
Write-Host "Built $stamp into $OutDir"
Get-ChildItem $OutDir | Format-Table Name, Length
