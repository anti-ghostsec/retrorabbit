# Run RetroRabbit on silent test media, flip channels, take screenshots.
param([string]$Dir, [string]$Media, [string]$Out)
# CI runners have no GPU: Mesa's software OpenGL (copied next to the exe by the
# workflow, for the test only) stands in for a graphics driver.
$env:GALLIUM_DRIVER = "llvmpipe"
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force -Path $Out | Out-Null
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
# mpv works on its own? (Also gets the virus scan of a new mpv.exe out of the way.)
& (Join-Path $Dir "mpv.exe") --version | Select-Object -First 1
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
if (-not $p.HasExited) { Stop-Process -Id $p.Id }
Get-Process mpv -ErrorAction SilentlyContinue | Stop-Process
