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
$p = Start-Process -FilePath (Join-Path $Dir "retrorabbit.exe") -ArgumentList "--channel", "2", "`"$Media`"" -PassThru
$ws = New-Object -ComObject WScript.Shell
Shot "1-start" 20
if ($p.HasExited) { Write-Error "RetroRabbit exited with code $($p.ExitCode)" }
$null = $ws.AppActivate($p.Id)
$ws.SendKeys("{PGUP}"); Shot "2-channel-up" 8
$ws.SendKeys("{PGDN}"); Shot "3-channel-down" 8
$ws.SendKeys("5");      Shot "4-typed-5" 8
$ws.SendKeys("g");      Shot "5-guide" 5
if (-not $p.HasExited) { Stop-Process -Id $p.Id }
Get-Process mpv -ErrorAction SilentlyContinue | Stop-Process
