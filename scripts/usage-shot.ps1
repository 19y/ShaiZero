# Drive the settings window for the README usage picture: open it on the dial
# page, photograph it; then maximize and restore to reproduce the repaint bug,
# photographing each state so the fix can be judged from pixels.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\ShaiZero.exe")
)
$ErrorActionPreference = "Stop"
. "$PSScriptRoot\testkit.ps1"
Add-Type -AssemblyName System.Drawing
if (-not ("Win.Shot" -as [type])) {
  Add-Type -Namespace Win -Name Shot -MemberDefinition @'
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Win.Shot.RC r);
    [DllImport("user32.dll")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    public struct RC { public int L, T, R, B; }
'@
}
$dir = Join-Path $env:TEMP "shaizero-usage"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Remove-Item "$dir\*.png" -ErrorAction SilentlyContinue

function Snap([IntPtr]$h, [string]$name) {
  $r = New-Object Win.Shot+RC
  [void][Win.Shot]::GetWindowRect($h, [ref]$r)
  $w = $r.R - $r.L; $t = $r.B - $r.T
  $bmp = New-Object System.Drawing.Bitmap $w, $t
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $g.GetHdc()
  [void][Win.Shot]::PrintWindow($h, $hdc, 2)
  $g.ReleaseHdc($hdc); $g.Dispose()
  $bmp.Save("$dir\$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Host "shot: $dir\$name.png ($w x $t)"
}

$sandbox = New-Sandbox (Join-Path $env:TEMP "shaizero-usage-state") ""
$proc = Start-Shaiwin $Exe $sandbox
try {
  Wait-Until { Get-AppWindow } 5000 | Out-Null
  $app = Get-AppWindow
  if (-not $app) { Fail "app window not found" }
  Invoke-Wm $app 0x800F ([IntPtr]1) ([IntPtr]::Zero)   # WM_APP+15 open settings on the dial page (wparam = section)
  Start-Sleep -Milliseconds 1200
  $set = Get-Settings
  if (-not $set) { Fail "settings window not found" }
  Snap $set "1-open"
  [void][Win.Shot]::SendMessageW($set, 0x0112, [IntPtr]0xF030, [IntPtr]::Zero)  # SC_MAXIMIZE
  Start-Sleep -Milliseconds 800
  Snap $set "2-maximized"
  [void][Win.Shot]::SendMessageW($set, 0x0112, [IntPtr]0xF120, [IntPtr]::Zero)  # SC_RESTORE
  Start-Sleep -Milliseconds 800
  Snap $set "3-restored"
  [void][Win.Shot]::SendMessageW($set, 0x0112, [IntPtr]0xF030, [IntPtr]::Zero)  # SC_MAXIMIZE again
  Start-Sleep -Milliseconds 800
  Snap $set "4-maximized-again"
  [void][Win.Shot]::SendMessageW($set, 0x0112, [IntPtr]0xF120, [IntPtr]::Zero)  # SC_RESTORE again
  Start-Sleep -Milliseconds 800
  Snap $set "5-restored-again"
  Write-Host "OK: $dir"
} finally {
  Stop-Shaiwin
  Remove-Fixtures
}
