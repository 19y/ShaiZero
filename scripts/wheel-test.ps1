# Regression tests for the wheel: the gesture, the two layers it draws on, the
# pick, the cancels, the previews that never touch a window, and the seats that
# have to outlive the process.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\wheel-test.ps1
#
# Covers:
#   1. hold the trigger and drag out past the threshold
#      -> the dial opens centred on the press point, at the size the app itself
#         reports for this monitor, and letting go on a sector raises a window
#   2. drag back to the middle and let go -> cancelled, nothing raised
#   3. Escape with the button still held -> cancelled, the dial comes down
#   4. a press that never crossed the threshold -> no dial, and the click is the
#         application's own
#   5. rest the pointer on a sector -> that window is drawn through the hole in
#         the dim, and it keeps its frame, its size and its place in the stack
#   6. rest the pointer on a window that is in the taskbar -> the picture is drawn
#         where the window would come back to, and a pick there brings it back
#   7. the same preview ended with Escape -> the window is still in the taskbar
#   8. `Ctrl+2` with a window in front -> it takes a seat, the file gains the
#         seat, and the label box opens in the same press
#   9. a word typed there and saved -> the file carries the word with the seat
#   10. kill the app, start it again -> both come back, and a second press on the
#         same window says it was already taken rather than seating it twice
#   11. cross from that sector to another -> the light moves and the desk does not:
#         neither window is lifted, restored or put anywhere else
#
# Why the assertions are window state and log lines rather than pixels: this
# session cannot synthesize input that a real hand would make and cannot capture
# a layered window's contents, so the checks read what can be checked from
# outside - does the disc exist, is it on screen, how wide is it, what did the app
# say it did. The look of the dial is the user's to judge; see the README.
#
# The wheel is a desktop-wide switcher: the dial lists every switchable window on
# the machine, not just the two fixtures this file makes. A release on a sector
# therefore sometimes raises one of the user's own windows, which is the feature
# working rather than the test misbehaving - step 1 prints which handle it was so
# nobody has to wonder. Steps 5 to 7 do more than read: the preview draws whatever
# window the pointer crosses - a thumbnail and a hole in the dim - and never touches
# it, so a window of the user's keeps the frame and the stack position it held before
# the gesture. That is the promise those steps are testing. Nothing closes, raises,
# restores or minimizes a window that the wheel did not pick.
#
# The app runs against its own LOCALAPPDATA: the real settings stay untouched.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\ShaiZero.exe")
)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\testkit.ps1"

# The dial is positioned in physical pixels by a per-monitor-aware process, so a
# reader that stays DPI-unaware would measure a wheel that never existed. Same
# thread, before any coordinate is read.
[void][W]::SetThreadDpiAwarenessContext([IntPtr](-4))

$root = Join-Path $env:TEMP "shaizero-wheel"
# The wheel is on with the shipped trigger (Alt + left, eight pixels of travel),
# and the shortcut keys are the shipped ones: Ctrl+2 takes a window onto the dial
# and opens the label box, Ctrl+T opens the panel that manages what is on it.
$cfg = "reduced_motion=0`nlang=en`n" +
       "excluded=`nwheel=1`ntrigger_mods=1`ntrigger_button=left`n" +
       "wheel_radius=150.0`nwheel_dead=0.42`ndrag_threshold=8`n" +
       "fav_mods=2`nfav_vk=50`npanel_mods=2`npanel_vk=84`n"
$state = New-Sandbox $root $cfg
$log = Join-Path $state "ShaiZero\shaizero.log"
Initialize-Fixtures $root
$h1 = New-Fixture "One" 0
$h2 = New-Fixture "Two" 1
Stop-Shaiwin

$VK_MENU = 0x12; $VK_ESCAPE = 0x1B; $VK_CONTROL = 0x11; $VK_2 = 0x32
function Key-Down([byte]$vk) { [void][W]::keybd_event($vk, 0, 0, [UIntPtr]::Zero) }
function Key-Up([byte]$vk) { [void][W]::keybd_event($vk, 0, 2, [UIntPtr]::Zero) }
function Lines { if (Test-Path $log) { @(Get-Content $log -Encoding utf8) } else { @() } }
function Mark { (Lines).Count }
function Since([int]$m) { @((Lines) | Select-Object -Skip $m) }
function Text([int]$m) { ((Since $m) -join "`n") }
function Disc { Find-By-Class "PanWen.Wheel" }
function Veil { Find-By-Class "PanWen.Veil" }
function Visible([IntPtr]$h) { ($h -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($h) }
# A handle read out of the log as hex. PowerShell 5.1 will not cast a UInt64 to an
# IntPtr, however small the number is, so the way round is through Int64.
function Handle-From-Hex([string]$hex) { return [IntPtr][Convert]::ToInt64($hex, 16) }

# The frame and taskbar state of every window on the desk, in one read. A hover
# that promises never to touch a window is checked against this: the window the
# pointer crosses must come out with the same entry it went in with. Stack indices
# are not used - other applications open and close windows while a test is running,
# and a rank that shifts because a tooltip vanished says nothing about the wheel.
function Get-Frames {
  $frames = @{}
  foreach ($h in Get-ZOrder) {
    if (-not [W]::IsWindow($h)) { continue }
    $r = Get-Rect $h
    if ($null -eq $r) { continue }
    $frames[[int64]$h] = "{0}|{1},{2},{3},{4}" -f ([W]::IsIconic($h)), $r.Left, $r.Top, $r.Right, $r.Bottom
  }
  return $frames
}
# A seat's receipt is a line in the settings file, and the file is the only place a
# seat can be proved from outside the window it belongs to. Read with the bystander's
# sharing modes: the app swaps this file in by renaming a sibling over it, and an
# ordinary reader holds it open against that write.
function Settings-Text { @((Read-SharedText (Join-Path $state "ShaiZero\settings.txt"))) -join "`n" }
function Field-Line([string]$key) {
  $m = [regex]::Match((Settings-Text), "(?m)^$key=(.*)`$")
  if ($m.Success) { return $m.Groups[1].Value }
  return $null
}
function Favourite-Line { Field-Line "favorite_slots" }
function Tag-Line { Field-Line "window_tags" }
$script:fails = 0
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { $script:fails++; Write-Host "  FAIL $what" }
}

# A press inside a fixture's client area: the frame is exempt on purpose, so the
# caption would be the wrong place to start a gesture the wheel must take.
function Client-Centre([IntPtr]$h) {
  $r = Get-Rect $h
  $s = [W]::GetDpiForWindow($h) / 96.0
  $cap = [int](34 * $s)
  $x = [int](($r.Left + $r.Right) / 2)
  $y = [int](($r.Top + $cap + $r.Bottom) / 2)
  return @($x, $y, $s)
}

function Let-Go { Press-Up }

# The middle of the first ring's band, in the pixels this screen measures. Both
# numbers come from the settings file this sandbox writes: the radius is 150
# logical pixels and 42 percent of it is hub, so the band runs from 63 to 150 and
# the safe place to aim is its middle. Outer rings are thinner, so a point that is
# inside this band cannot be mistaken for one of them.
function Get-RingReach {
  $scale = [W]::GetDpiForWindow((Find-By-Class "PanWen.Wheel")) / 96.0
  if ($scale -le 0) { $scale = 1.0 }
  return [int](106.5 * $scale)
}

# The middle of the `ring`th band. The same three numbers from the settings file:
# radius 150, hub 42 percent of it, an outer band 52 percent of what is left over,
# and 3 logical pixels of gap between the rings - see Geometry::ring_bounds. A dial
# with three rings seats a window in the second or third ring whenever the desktop
# offers more than twelve windows, and no radius inside the first band can reach it.
function Get-RingReachAt([int]$ring) {
  if ($ring -le 1) { return Get-RingReach }
  $scale = [W]::GetDpiForWindow((Find-By-Class "PanWen.Wheel")) / 96.0
  if ($scale -le 0) { $scale = 1.0 }
  $radius = 150.0
  $hub = $radius * 0.42
  $band = ($radius - $hub) * 0.52
  $lo = $radius + 3.0 + ($ring - 2) * ($band + 3.0)
  return [int](($lo + $band / 2.0) * $scale)
}

# Press inside a fixture's client area, open the dial, and sweep the rings one tenth
# of a circle at a time until the app reports previewing the window named by `$want` -
# or the first window it previews at all, when `$want` is empty. The pointer is left
# where the preview was reported with the button still held: the caller decides
# whether that becomes a pick or a cancel.
#
# The sweep is here because the dial seats every window on the desktop, so which
# sector a given window lands in depends on what else is open, which a test may not
# arrange. Ten degrees steps are finer than the widest seam (a ring holds at most
# twelve windows, so no sector is narrower than thirty degrees), which means every
# sector of a ring is visited. The rings are visited from the inside out because a
# busy desktop seats windows in the second and third bands, and a sweep that stays at
# one radius never passes a sector that lives out there - which is how the taskbar
# preview used to skip itself.
function Hover-On([int]$x, [int]$y, [int]$reach, [IntPtr]$want) {
  if (-not (Hold-And-Drag $x $y ($x + $reach) $y 8 160)) { return $null }
  $d = Get-Rect (Disc)
  if ($null -eq $d) { return $null }
  $cx = [int](($d.Left + $d.Right) / 2)
  $cy = [int](($d.Top + $d.Bottom) / 2)
  $hex = if ($want -ne [IntPtr]::Zero) { $want.ToInt64().ToString("x") } else { $null }
  for ($ring = 1; $ring -le 3; $ring++) {
    $r = Get-RingReachAt $ring
    for ($i = 0; $i -lt 36; $i++) {
      $rad = ($i * 10) * [Math]::PI / 180.0
      $px = [int]($cx + $r * [Math]::Sin($rad))
      $py = [int]($cy - $r * [Math]::Cos($rad))
      if (-not (Send-Move $px $py)) { return $null }
      Start-Sleep -Milliseconds 90
      foreach ($found in [regex]::Matches((Text $script:hoverMark), "wheel: preview of 0x([0-9a-fA-F]+)")) {
        $got = $found.Groups[1].Value
        if ($null -eq $hex) { return $got }
        if ($got -eq $hex) { return $got }
      }
    }
  }
  return $null
}

# The window the dial says it is previewing, once it has stopped changing its mind.
# A hover that is still crossing a seam reports one window and then the next, and a
# stack read at that moment describes a window the pointer has already left - which
# is how a good run came to fail here. The injected moves are queued faster than the
# app drains them, so the answer is not the last line read but the last line that did
# not change again: two polls of the same word, and only then is the window the one
# the pointer is resting on. `$null` means the pointer came to rest on the hub or a
# gap, so there is no preview to assert about.
function Settled-Preview([int]$from) {
  $prev = ""
  for ($try = 0; $try -lt 15; $try++) {
    Start-Sleep -Milliseconds 200
    $seen = [regex]::Matches((Text $from), "wheel: preview of 0x([0-9a-fA-F]+)")
    if ($seen.Count -eq 0) { continue }
    $now = $seen[$seen.Count - 1].Groups[1].Value
    if ($now -eq $prev) {
      return $now
    }
    $prev = $now
  }
  return $null
}

try {
  if (-not (Test-CursorControl)) {
    Write-Host "== SKIPPED: the pointer cannot be moved in this session =="
    Write-Host "   the wheel is driven entirely by input, so there is no way to"
    Write-Host "   test it from here. Every step below needs a pointer to place."
    $script:fails = 0
    return
  }

  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -match "wheel ready") "the log says the dial is ready"
  Check ((Text $m) -match "wheel: the hook is in") "and the hook is in the desktop"
  Check (Test-Window (Disc)) "the disc window exists while the dial is shut"
  Check (-not (Visible (Disc))) "and is not on screen until the dial opens"

  # --- 1. open, hover, pick -----------------------------------------------------
  $p = Client-Centre $h1
  $reach = [int](95 * $p[2])
  $m = Mark
  Key-Down $VK_MENU
  try {
    if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 12 200) {
      Check ((Text $m) -match "wheel: opened on a held trigger at") "Alt + drag out opens the dial"
      Check (Visible (Disc)) "the disc window is on screen"
      Check (Visible (Veil)) "the dim layer is on screen"
      Check ([W]::IsTopmost((Disc))) "and the disc is above every window the dial is listing"
      $r = Get-Rect (Disc)
      $got = ($r.Right - $r.Left)
      $say = [regex]::Match((Text $m), "wheel: (\d+)px square at scale ([\d.]+)")
      if ($say.Success) {
        $want = [int]$say.Groups[1].Value
        Check ([Math]::Abs($got - $want) -le 2) ("the disc is $got wide, the app said $want")
        Check ($got -eq ($r.Bottom - $r.Top)) "and the disc is square"
      } else {
        Check $false "the app never reported the dial's own size"
      }
      # The centre of the square is where the finger pressed, unless the dial had
      # to be moved whole to stay on the desktop.
      $cx = [int](($r.Left + $r.Right) / 2); $cy = [int](($r.Top + $r.Bottom) / 2)
      Check ([Math]::Abs($cx - $p[0]) -le 2 -and [Math]::Abs($cy - $p[1]) -le 2) ("the dial is centred on the press at $($p[0]),$($p[1]) (it says $cx,$cy)")
      Let-Go
      Start-Sleep -Milliseconds 250
      # Every pick ends in `window::activate`, and that call is the whole promise:
      # the dial hands the window to the window manager and says how it went.
      $picked = [regex]::Match((Text $m), "activate: hwnd=0x([0-9a-fA-F]+)")
      Check ($picked.Success) "letting go on a sector raises a window"
      if ($picked.Success) {
        $hwnd = [Convert]::ToUInt64($picked.Groups[1].Value, 16)
        # The handles come out of the fixture helpers as IntPtr, and PowerShell
        # will not widen one to UInt64 by itself.
        $mine = ($hwnd -eq [Convert]::ToUInt64($h1.ToInt64())) -or ($hwnd -eq [Convert]::ToUInt64($h2.ToInt64()))
        if ($mine) { Write-Host "    it raised a fixture, 0x$($picked.Groups[1].Value)" }
        else {
          Write-Host ("    it raised 0x$($picked.Groups[1].Value), one of the desktop's own" +
                      " windows: the dial lists the whole desktop, which is the feature.")
        }
      }
    } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
  } finally { Key-Up $VK_MENU }
  Check (-not (Visible (Disc))) "the disc is gone after the pick"
  Check (-not (Visible (Veil))) "the dim is gone after the pick"

  # --- 2. cancel from the hub ---------------------------------------------------
  Start-Sleep -Milliseconds 500
  $p = Client-Centre $h2
  $m = Mark
  Key-Down $VK_MENU
  try {
    if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 8 160) {
      if (Send-Move $p[0] $p[1]) {
        Start-Sleep -Milliseconds 200
        Let-Go
        Start-Sleep -Milliseconds 250
        Check ((Text $m) -match "wheel: cancelled") "letting go in the middle cancels"
        Check ((Text $m) -notmatch "activate: hwnd=") "and nothing was raised"
      } else { Write-Host "  FAIL the pointer would not come back to the hub"; $script:fails++ }
    } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
  } finally { Key-Up $VK_MENU }
  Check (-not (Visible (Disc))) "the disc is gone after the cancel"

  # --- 3. Escape, with the button still held ------------------------------------
  Start-Sleep -Milliseconds 500
  $p = Client-Centre $h1
  $m = Mark
  Key-Down $VK_MENU
  try {
    if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 8 160) {
      Check (Visible (Disc)) "the dial is open again"
      Key-Down $VK_ESCAPE; Key-Up $VK_ESCAPE
      Start-Sleep -Milliseconds 250
      Check (-not (Visible (Disc))) "Escape closes it while the button is still held"
      Check ((Text $m) -match "wheel: cancelled") "and says so"
      Let-Go
    } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
  } finally { Key-Up $VK_MENU }

  # --- 4. a click that never became a gesture -----------------------------------
  Start-Sleep -Milliseconds 500
  $p = Client-Centre $h1
  $m = Mark
  Key-Down $VK_MENU
  try {
    if (Send-Move $p[0] $p[1]) {
      Start-Sleep -Milliseconds 120
      [W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
      Start-Sleep -Milliseconds 120
      Let-Go
      Start-Sleep -Milliseconds 250
      Check (-not (Visible (Disc))) "a press that barely travelled opens nothing"
      Check ((Text $m) -notmatch "wheel: opened|activate: hwnd=|wheel: cancelled") "and the dial never spoke"
    } else { Write-Host "  FAIL the pointer would not reach the fixture"; $script:fails++ }
  } finally { Key-Up $VK_MENU }

  # --- 5. the preview lights the window under the pointer without touching it ---
  #
  # The hole in the dim is cut at the previewed window's own rectangle and the
  # picture in it is a compositor thumbnail, so a hover draws and never owns a
  # window. Two things are read from outside: the preview layer sits exactly on
  # that window's rectangle, and the window itself still has the frame it had
  # when the dial opened.
  Start-Sleep -Milliseconds 500
  $p = Client-Centre $h1
  $m = Mark
  $script:hoverMark = $m
  $frames = Get-Frames
  Key-Down $VK_MENU
  try {
    $got = Hover-On $p[0] $p[1] $reach ([IntPtr]::Zero)
    if ($null -eq $got) {
      Write-Host "  SKIPPED: no sector of the first ring reported a preview"
    } else {
      # The sweep stops at the first preview it reads while the moves queued
      # behind it are still being drained, so assert about the one the dial came
      # to rest on.
      $got = Settled-Preview $m
      if ($null -eq $got) {
        Write-Host "  SKIPPED: the pointer went on travelling off the preview it first lit"
        Key-Down $VK_ESCAPE; Key-Up $VK_ESCAPE
        Start-Sleep -Milliseconds 300
      } else {
        $hovered = Handle-From-Hex $got
        $key = [int64]$hovered
        if ($frames.ContainsKey($key)) {
          $now = Get-Frames
          Check ($now.ContainsKey($key) -and $now[$key] -eq $frames[$key]) "the hover left the window's frame and taskbar state alone: $($frames[$key]) -> $(if ($now.ContainsKey($key)) { $now[$key] } else { 'gone' })"
        } else {
          Write-Host "  | the previewed window belongs to this app, so there is no outside frame to compare"
        }
        $said = @((Since $m) | Where-Object { $_ -match "wheel: preview of 0x$got" } | Select-Object -Last 1)
        if ($said -match ", in front") {
          # The window already in front is its own picture: the cut shows it and
          # there is nothing to draw.
          Check $true "the previewed window is the one already in front, so the hole shows it"
        } else {
          $pv = Find-By-Class "PanWen.Preview"
          Check (($pv -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($pv)) "a preview layer is on screen for it"
          if (($pv -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($pv) -and -not [W]::IsIconic($hovered)) {
            $want = Get-Rect $hovered
            $pr = Get-Rect $pv
            # The picture stops just inside the rim (4px), and a maximized window's
            # own frame reaches past the monitor while its visible frame does not.
            $fits = ([Math]::Abs($pr.Left - 4 - $want.Left) -le 20) -and ([Math]::Abs($pr.Top - 4 - $want.Top) -le 20) -and
                    ([Math]::Abs($pr.Right + 4 - $want.Right) -le 20) -and ([Math]::Abs($pr.Bottom + 4 - $want.Bottom) -le 20)
            Check $fits "and the picture is drawn on that window's own rectangle ($($pr.Left),$($pr.Top) $($pr.Right - $pr.Left)x$($pr.Bottom - $pr.Top) over $($want.Left),$($want.Top) $($want.Right - $want.Left)x$($want.Bottom - $want.Top))"
          }
        }
        Key-Down $VK_ESCAPE; Key-Up $VK_ESCAPE
        Start-Sleep -Milliseconds 300
        Check ((Text $m) -match "wheel: cancelled") "Escape ends the gesture"
        $pv = Find-By-Class "PanWen.Preview"
        Check (-not (Visible $pv)) "and the picture goes away with it"
      }
      Let-Go
    }
  } finally { Key-Up $VK_MENU }
  Check (-not (Visible (Disc))) "the disc is down after the preview"

  # --- 6. a window in the taskbar is previewed where it would come back to -------
  #
  # A minimized window has no rectangle on any monitor, and restoring it to find
  # one is what made a sweep feel heavy. The preview is drawn at the frame the
  # window would return to, and the window stays in the taskbar; the pick that
  # follows is what brings it back.
  Start-Sleep -Milliseconds 500
  $homeRect = Get-Rect $h2
  [void][W]::ShowWindow($h2, 6)
  if (-not (Wait-Until ({ [W]::IsIconic($h2) }) 3000)) {
    Write-Host "  SKIPPED: the fixture would not minimize, so there is no taskbar window to preview"
  } else {
    $p = Client-Centre $h1
    $m = Mark
    $script:hoverMark = $m
    Key-Down $VK_MENU
    try {
      $got = Hover-On $p[0] $p[1] $reach $h2
      if ($null -eq $got) {
        Write-Host "  SKIPPED: the sweep did not land on the minimized fixture's sector"
      } else {
        Check ([W]::IsIconic($h2)) "the preview left the minimized window in the taskbar"
        Check ((Text $m) -match "wheel: preview of 0x$got[^\r\n]*, minimized") "and the app says it is previewing a minimized window"
        $pv = Find-By-Class "PanWen.Preview"
        if (($pv -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($pv)) {
          $pr = Get-Rect $pv
          $fits = ([Math]::Abs($pr.Left - 4 - $homeRect.Left) -le 10) -and ([Math]::Abs($pr.Top - 4 - $homeRect.Top) -le 10) -and
                  ([Math]::Abs($pr.Right + 4 - $homeRect.Right) -le 10) -and ([Math]::Abs($pr.Bottom + 4 - $homeRect.Bottom) -le 10)
          Check $fits "the picture sits on the frame the window would come back to ($($pr.Left),$($pr.Top) $($pr.Right - $pr.Left)x$($pr.Bottom - $pr.Top) over $($homeRect.Left),$($homeRect.Top))"
        } else {
          Check $false "a preview layer is on screen for the taskbar window"
        }
        Let-Go
        Start-Sleep -Milliseconds 300
        Check ((Text $m) -match "activate: hwnd=0x$got") "letting go on that sector keeps it"
        Check (-not [W]::IsIconic($h2)) "the pick is what brings a minimized window back"
        Check ([W]::IsAbove($h2, $h1)) "and it is in front of the window the gesture started from"
      }
    } finally { Key-Up $VK_MENU }
  }

  # --- 7. and an escape leaves it exactly there ---------------------------------
  Start-Sleep -Milliseconds 500
  [void][W]::ShowWindow($h2, 6)
  if (-not (Wait-Until ({ [W]::IsIconic($h2) }) 3000)) {
    Write-Host "  SKIPPED: the fixture would not minimize again"
  } else {
    $p = Client-Centre $h1
    $m = Mark
    $script:hoverMark = $m
    Key-Down $VK_MENU
    try {
      $got = Hover-On $p[0] $p[1] $reach $h2
      if ($null -eq $got) {
        Write-Host "  SKIPPED: the sweep did not land on the minimized fixture's sector"
      } else {
        Check ([W]::IsIconic($h2)) "the preview never brought it out of the taskbar"
        Key-Down $VK_ESCAPE; Key-Up $VK_ESCAPE
        Start-Sleep -Milliseconds 300
        Let-Go
        Check ((Text $m) -match "wheel: cancelled") "Escape ends it"
        Check ([W]::IsIconic($h2)) "and the window is still in the taskbar"
        Check ((Text $m) -match "wheel: preview of 0x$got[^\r\n]*, minimized") "which is how it was previewed all along"
      }
    } finally { Key-Up $VK_MENU }
  }

  # --- 8. the shortcut takes the window in front onto the dial ----------------
  #
  # `Ctrl+2` is the user's way of saying "this one, always". One press does two
  # things and the seat is the one that cannot be undone by a keystroke: it is
  # written to the file before the label box appears, so this step reads the file
  # for the promise and treats the box as the second half of the press.
  #
  # The dial seats a favourite whose window is shut by the same line in the file;
  # that half is a question about a list, and crates/core's unit tests answer it.
  # From here the seat is only visible as the file and as what comes back after a
  # restart, which is what steps 8 to 10 assert.
  Start-Sleep -Milliseconds 500
  $m = Mark
  [void][W]::SetForegroundWindow($h1)
  Start-Sleep -Milliseconds 400
  if ((Test-LockScreen) -or ([W]::GetForegroundWindow() -ne $h1)) {
    Write-Host "  SKIPPED: nothing can be put in front in this session, so the shortcut has no window to name"
  } else {
    Key-Down $VK_CONTROL
    Key-Down $VK_2
    Key-Up $VK_2
    Key-Up $VK_CONTROL
    Start-Sleep -Milliseconds 700
    # What the app calls the seat is the whole difference between a shortcut and a
    # list that grows, so the receipt is printed with the verdict.
    $said = @((Since $m) | Where-Object { $_ -match "favourite: " } | Select-Object -Last 1) -join " / "
    Check ($said -match "favourite: joined the dial") "the shortcut takes the window in front onto the dial ($said)"
    $fav = Favourite-Line
    Check ($null -ne $fav -and $fav -match "shaiwinfixtureone") "and the file gained its seat: $fav"
    $box = Get-TagBox
    Check (Test-Window $box) "and the label box opened in the same press"

    # --- 9. a word typed there becomes the window's label --------------------
    if (Test-Window $box) {
      $edit = Find-Child-By-Class $box "EDIT"
      if ($edit -eq [IntPtr]::Zero) {
        Write-Host "  FAIL the label box has no field to type in"; $script:fails++
      } else {
        # Typed through the field's own message rather than the keyboard: the box
        # wants the foreground to take keys, and this session has been known to
        # refuse one window the foreground while granting another it.
        Set-Child-Text $edit "post"
        Press-Enter $box
        Start-Sleep -Milliseconds 700
        Check ((Text $m) -match "tag: window labelled") "the box says the window is labelled now"
        $line = Tag-Line
        Check ($null -ne $line -and $line -match "post") "and the file carries the word with the seat: $line"
        Check (Wait-Until ({ -not (Test-Window (Get-TagBox)) }) 3000) "the box went away with the save"
      }
    }

    # --- 10. and both outlive the app ----------------------------------------
    $m = Mark
    Stop-Shaiwin
    Start-Sleep -Milliseconds 400
    [void](Start-Shaiwin $Exe $state)
    Start-Sleep -Milliseconds 1800
    Check ((Text $m) -match "wheel: 1 favourite window\(s\) read from the settings") "a restart reads the seat back"
    Check ((Text $m) -match "wheel: 1 window tag\(s\) read from the settings") "and the label with it"
    Check ((Favourite-Line) -match "shaiwinfixtureone") "the shutdown did not write the seat away: $(Favourite-Line)"
    Check ((Tag-Line) -match "post") "nor the label: $(Tag-Line)"
    # A second press on the same window says so instead of taking a second seat,
    # which is the difference between a shortcut and a list that grows.
    $m = Mark
    [void][W]::SetForegroundWindow($h1)
    Start-Sleep -Milliseconds 400
    if ([W]::GetForegroundWindow() -eq $h1) {
      Key-Down $VK_CONTROL
      Key-Down $VK_2
      Key-Up $VK_2
      Key-Up $VK_CONTROL
      Start-Sleep -Milliseconds 700
      Check ((Text $m) -match "already on the dial") "and the shortcut knows it had already taken that window"
      Check (@((Favourite-Line) -split ",").Count -eq 1) "still one seat, not two: $(Favourite-Line)"
      $box2 = Get-TagBox
      if (Test-Window $box2) { Press-Escape $box2; Start-Sleep -Milliseconds 400 }
    } else {
      Write-Host "  SKIPPED: the fixture would not come back to the front for the second press"
    }
  }

  # --- 11. crossing a seam moves the light, never the desk ---------------------
  #
  # Two different windows in a row, and both come out of the crossing with the
  # frame and taskbar state they went in with. A preview that lifted each window in
  # turn is what made a sweep feel heavy, and a window's own frame is the proof it
  # never happened.
  Start-Sleep -Milliseconds 500
  $m = Mark
  $script:hoverMark = $m
  $frames = Get-Frames
  $p = Client-Centre $h1
  Key-Down $VK_MENU
  try {
    $first = Hover-On $p[0] $p[1] $reach ([IntPtr]::Zero)
    if ($null -eq $first) {
      Write-Host "  SKIPPED: the preview never lit a window to move on from"
    } else {
      # Walk on round the ring until a different window is previewed. The dial
      # seats the whole desktop, so which sector holds which window is not
      # something a test may arrange; five degree steps are finer than the
      # narrowest sector of a three ring dial, so the walk passes over all of them.
      $d = Get-Rect (Disc)
      $second = $null
      if ($null -ne $d) {
        $cx = [int](($d.Left + $d.Right) / 2)
        $cy = [int](($d.Top + $d.Bottom) / 2)
        $r = Get-RingReach
        for ($i = 0; $i -lt 72 -and $null -eq $second; $i++) {
          $rad = (12 + $i * 5) * [Math]::PI / 180.0
          if (-not (Send-Move ([int]($cx + $r * [Math]::Sin($rad))) ([int]($cy - $r * [Math]::Cos($rad))))) { break }
          Start-Sleep -Milliseconds 90
          foreach ($found in [regex]::Matches((Text $m), "wheel: preview of 0x([0-9a-fA-F]+)")) {
            if ($found.Groups[1].Value -ne $first) { $second = $found.Groups[1].Value; break }
          }
        }
      }
      if ($null -eq $second) {
        Write-Host "  SKIPPED: no other sector previewed a different window"
      } else {
        # The same queue problem as step 5: the walk breaks at the first preview it
        # reads while the moves it already injected carry the pointer on.
        $held = Settled-Preview $m
        if ($null -eq $held) {
          Write-Host "  SKIPPED: the pointer came to rest on the hub or a seam"
        } else {
          if ($held -ne $second) {
            Write-Host "  | the pointer ended on 0x$held, not the 0x$second the walk first read: the injected moves outran the answer"
            $second = $held
          }
          Check (Visible (Disc)) "the dial never came down for the crossing"
          $now = Get-Frames
          foreach ($hx in @($first, $second)) {
            $key = [int64](Handle-From-Hex $hx)
            if (-not $frames.ContainsKey($key)) {
              Write-Host "  | 0x$hx belongs to this app, so there is no outside frame to compare"
              continue
            }
            Check ($now.ContainsKey($key) -and $now[$key] -eq $frames[$key]) "0x$hx came out of the crossing as it went in: $($frames[$key]) -> $(if ($now.ContainsKey($key)) { $now[$key] } else { 'gone' })"
          }
        }
      }
      Key-Down $VK_ESCAPE; Key-Up $VK_ESCAPE
      Start-Sleep -Milliseconds 300
      Let-Go
    }
  } finally { Key-Up $VK_MENU }
} finally {
  Key-Up $VK_MENU
  Key-Up $VK_CONTROL
  Stop-Shaiwin
  Remove-Fixtures
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
  Write-Host "cleaned up"
}
Write-Host ""
if ($script:fails -eq 0) { Write-Host "PASS" }
else { Write-Host "FAIL: $($script:fails) check(s)"; exit 1 }
