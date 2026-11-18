# Run RetroRabbit on silent test media, flip channels, take screenshots.
param([string]$Dir, [string]$Media, [string]$Out)
# CI runners have no GPU: Mesa's software OpenGL (copied next to the exe by the
# workflow, for the test only) stands in for a graphics driver.
$env:GALLIUM_DRIVER = "llvmpipe"
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Out | Out-Null
# The runner's screen is 1024x768, smaller than RetroRabbit's window.
try { Set-DisplayResolution -Width 1920 -Height 1080 -Force -ErrorAction Stop } catch { "couldn't change the screen size: $_" }
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
function Shot($name, $wait) {
  Start-Sleep -Seconds $wait
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
  $bmp.Save((Join-Path $Out "$name.png"))
  $g.Dispose(); $bmp.Dispose()
}
# Does mpv run with RetroRabbit's options? (Also gets the virus scan of a new
# mpv.exe out of the way.) mpv.com is the console twin of mpv.exe.
$mpvcom = Join-Path $Dir "mpv.com"
if (Test-Path $mpvcom) {
  $pipe = "\\.\pipe\rr-smoke-test"
  $opts = @("--input-ipc-server=$pipe", "--idle=yes", "--keep-open=no", "--config=no", "--load-scripts=no",
    "--ytdl=no", "--input-default-bindings=no", "--input-vo-keyboard=no", "--input-cursor=no",
    "--cache-on-disk=no", "--resume-playback=no", "--save-position-on-quit=no",
    "--audio-client-name=RetroRabbit", "--term-osd=no", "--force-window=no", "--cache=yes",
    "--demuxer-max-bytes=64MiB", "--demuxer-max-back-bytes=16MiB", "--network-timeout=20",
    "--hwdec=auto-safe", "--hr-seek=no", "--osd-level=1", "--osd-font=monospace", "--volume=80", "--af=",
    "--msg-level=all=v")
  $m = Start-Process -FilePath $mpvcom -ArgumentList $opts -PassThru -NoNewWindow `
    -RedirectStandardOutput (Join-Path $Out "mpv-test.txt") -RedirectStandardError (Join-Path $Out "mpv-test-err.txt")
  Start-Sleep -Seconds 8
  "mpv test: running=$(-not $m.HasExited) exit=$($m.ExitCode) pipe=$(Test-Path $pipe)"
  if (-not $m.HasExited) { Stop-Process -Id $m.Id }
  Get-Content (Join-Path $Out "mpv-test.txt"), (Join-Path $Out "mpv-test-err.txt") -ErrorAction SilentlyContinue | Select-Object -Last 30
}
$p = Start-Process -FilePath (Join-Path $Dir "retrorabbit.exe") -ArgumentList "--channel", "2", "`"$Media`"" -PassThru `
  -RedirectStandardError (Join-Path $Out "app-stderr.txt") -RedirectStandardOutput (Join-Path $Out "app-stdout.txt")
$ws = New-Object -ComObject WScript.Shell
# Which OpenGL is it loading? (Mesa's opengl32.dll next to the exe, we hope.)
for ($i = 0; $i -lt 30 -and -not $p.HasExited; $i++) {
  Start-Sleep -Milliseconds 100
  try {
    $gl = (Get-Process -Id $p.Id).Modules | Where-Object { $_.ModuleName -match "opengl|gallium|glapi|EGL" }
    if ($gl) { $gl | ForEach-Object { "loaded: $($_.FileName)" }; break }
  } catch {}
}
Shot "1-start" 20
if ($p.HasExited) {
  Get-Content (Join-Path $Out "app-stderr.txt") -ErrorAction SilentlyContinue
  Write-Error "RetroRabbit exited with code $($p.ExitCode)"
}
$null = $ws.AppActivate($p.Id)
$ws.SendKeys("{PGUP}"); Shot "2-channel-up" 8
$ws.SendKeys("{PGDN}"); Shot "3-channel-down" 8
$ws.SendKeys("5");      Shot "4-typed-5" 8
$ws.SendKeys("g");      Shot "5-guide" 5
"mpv processes: $((Get-Process mpv -ErrorAction SilentlyContinue | Measure-Object).Count)"
if (-not $p.HasExited) { Stop-Process -Id $p.Id }
Get-Process mpv -ErrorAction SilentlyContinue | Stop-Process
exit 0
