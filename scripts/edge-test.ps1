# Edge cases for the dial: the screen edges it has to stay inside, the frame it
# must leave alone, and the timer that brings it down when the hand stops.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\edge-test.ps1
#
# Covers:
#   1. a press near the top-left corner -> the dial says it moved and comes up
#      whole inside the desktop
#   2. a press near the bottom-right corner -> the same, from the other side
#   3. a trigger drag that starts on a window's caption -> the dial never opens
#      and the window is dragged where the hand took it: the non-client frame
#      belongs to the application and to Windows, not to the wheel
#   4. the gesture held with nothing moving -> after the watchdog's ten seconds
#      the dial closes itself, because a dial left on the desktop is a screen the
#      user cannot see through
#
# Steps 1, 2 and 4 hold the gesture open on purpose and read the disc window's own
# rectangle; step 3 reads the fixture's. All four are the cases a happy-path drag
# in scripts\wheel-test.ps1 does not reach.
#
# The windows are throwaway fixtures started by this script, positioned by this
# script, so nothing the user was working on is moved; and the app runs against its
# own LOCALAPPDATA, so the real settings stay untouched.
param([string]$Exe = (Join-Path $PSScriptRoot "..\target\release\ShaiZero.exe"))

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\testkit.ps1"

$root = Join-Path $env:TEMP "shaizero-edge"
# The shipped trigger (Alt + left drag, eight pixels of travel) and a dial big
# enough that a press sixteen pixels from a corner cannot fit one on that side.
$cfg = "reduced_motion=0`nlang=en`nexcluded=`nwheel=1`ntrigger_mods=1`ntrigger_button=left`n" +
       "wheel_radius=150.0`nwheel_dead=0.42`ndrag_threshold=8`nfav_mods=2`nfav_vk=50`npanel_mods=2`npanel_vk=84`n"
$state = New-Sandbox $root $cfg
$log = Join-Path $state "ShaiZero\shaizero.log"
Initialize-Fixtures $root

$VK_MENU = 0x12; $VK_ESCAPE = 0x1B
function Lines { @(Read-SharedText $log) }
function Mark { (Lines).Count }
function Since([int]$m) { @((Lines) | Select-Object -Skip $m) }
function Text([int]$m) { (Since $m) -join "`n" }
# The app writes its log as the thing happens, but the line reaches the file a few
# hundred milliseconds later, so a check that reads immediately is asking the log
# to be faster than it is. Waiting for the line is what these steps mean.
function Said([int]$m, [string]$pattern) {
  return (Wait-Until ({ @((Since $m) -join "`n") -match $pattern }) 4000)
}
function Disc { Find-By-Class "PanWen.Wheel" }
function Visible([IntPtr]$h) { ($h -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($h) }
$script:fails = 0
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { $script:fails++; Write-Host "  FAIL $what" }
}
function Key-Down([byte]$vk) { [void][W]::keybd_event($vk, 0, 0, [UIntPtr]::Zero) }
function Key-Up([byte]$vk) { [void][W]::keybd_event($vk, 0, 2, [UIntPtr]::Zero) }

# Put a fixture exactly where a step needs it, without activating it: the press
# has to land where the corner is, and a window that comes forward when moved
# would take the foreground from the test.
# 0x0004 no z-order, 0x0010 no activation.
function Move-To([IntPtr]$h, [int]$x, [int]$y, [int]$w, [int]$ht) {
  [void][W]::SetWindowPos($h, [IntPtr]::Zero, $x, $y, $w, $ht, 0x0004 -bor 0x0010)
  Start-Sleep -Milliseconds 350
  $r = Get-Rect $h
  return ($r -and $r.Left -le $x + 4 -and $r.Top -le $y + 4)
}

# The dial has to be whole: every pixel of the square inside the desktop the user
# is looking at. Whether it *moved* is proved by the arithmetic rather than by
# which monitor this session has: the press is nearer this edge than the square's
# own half-width, so a square centred on the press could not have fitted, and a
# square that fits is a square that was picked up and put down somewhere else.
function Check-Moved([int]$pressX, [int]$pressY, [string]$where) {
  $d = Get-Rect (Disc)
  $v = Get-VirtualBounds
  if ($null -eq $d) { Write-Host "  FAIL there is no disc to measure $where"; $script:fails++; return }
  $half = [int](($d.Right - $d.Left) / 2)
  $say = "the whole dial is inside the desktop $where ($($d.Left),$($d.Top)-$($d.Right),$($d.Bottom) of the desktop's $($v.Left),$($v.Top)-$($v.Right),$($v.Bottom))"
  Check ($d.Left -ge $v.Left -and $d.Top -ge $v.Top -and $d.Right -le $v.Right -and $d.Bottom -le $v.Bottom) $say
  $closest = ($pressX - $v.Left), ($v.Right - $pressX), ($pressY - $v.Top), ($v.Bottom - $pressY) |
    Sort-Object | Select-Object -First 1
  Check ($closest -lt $half) "and it had to move: the press is $closest px from the nearest edge, half the dial is $half"
}

try {
  if (-not (Test-CursorControl)) {
    Write-Host "== SKIPPED: the pointer cannot be moved in this session =="
    Write-Host "   every step here is a gesture, and a gesture needs a pointer to place"
    return
  }

  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -match "wheel: the hook is in") "the dial is ready and the hook is in the desktop"

  $a = New-Fixture "Edge" 0
  $s = [W]::GetDpiForWindow($a) / 96.0
  $wide = [int](460 * $s)
  $high = [int](320 * $s)
  $v = Get-VirtualBounds

  # --- 1. the top-left corner ---------------------------------------------------
  # The press is near the corner of the desktop and inside the fixture's client.
  # Both halves matter: the corner is where a dial of 150 logical pixels has no
  # room at all, and the client is the only part of a window the dial is allowed
  # to take - a press in the frame belongs to Windows, which is step 3.
  if (-not (Move-To $a ($v.Left + 8) ($v.Top + 8) $wide $high)) {
    Write-Host "  FAIL the fixture would not go to the top-left corner"
  } else {
    $cb = Get-ClientBounds $a
    $x = $cb.Left + 8
    $y = $cb.Top + 8
    $m = Mark
    Key-Down $VK_MENU
    try {
      if (Hold-And-Drag $x $y ($x + [int](110 * $s)) ($y + [int](110 * $s)) 10 180) {
        Check (Visible (Disc)) "a press in the top-left corner opens the dial"
        Check (Said $m "near an edge, so the whole dial moved") "and the app says it moved to stay whole"
        Check (-not ((Text $m) -match "was on a window's frame")) "and the press at $x,$y was in the client, not in the frame"
        Check-Moved $x $y "at the top-left"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      Key-Up $VK_MENU
    }
  }

  # --- 2. the bottom-right corner -----------------------------------------------
  if (-not (Move-To $a ($v.Right - $wide - 8) ($v.Bottom - $high - 8) $wide $high)) {
    Write-Host "  FAIL the fixture would not go to the bottom-right corner"
  } else {
    $cb = Get-ClientBounds $a
    $x = $cb.Right - 8
    $y = $cb.Bottom - 8
    $m = Mark
    Key-Down $VK_MENU
    try {
      if (Hold-And-Drag $x $y ($x - [int](110 * $s)) ($y - [int](110 * $s)) 10 180) {
        Check (Visible (Disc)) "a press in the bottom-right corner opens the dial"
        Check (Said $m "near an edge, so the whole dial moved") "and it moved whole again"
        Check (-not ((Text $m) -match "was on a window's frame")) "and the press at $x,$y was in the client, not in the frame"
        Check-Moved $x $y "at the bottom-right"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      Key-Up $VK_MENU
    }
  }

  # --- 3. the caption is not the dial's to take ---------------------------------
  #
  # The exemption covers the whole non-client frame, so a drag that starts on the
  # caption is Windows' own move-the-window gesture with the trigger modifier held
  # over it. Three halves are asserted: the dial stayed shut, the app said why in
  # its own words, and the window moved where the hand took it.
  #
  # The press has to be on the fixture's caption for any of that to mean anything,
  # and a fixture only owns a pixel if nothing is drawn over it - so it is put at
  # the head of the stack and the stack is read back before the hand moves.
  [void][W]::SetWindowPos($a, [IntPtr]::Zero, ($v.Left + 320), ($v.Top + 300), $wide, $high, 0x0004 -bor 0x0010)
  # Asking for the head of the band is not the same as getting it: this is another
  # process's window, and Windows answers a z-order or activation request from a
  # process that does not hold the foreground on the foreground's terms - so some
  # sessions give the fixture its pixel and some do not, and a step that only
  # sometimes presses a caption is a step that only sometimes says anything.
  # Topmost is the one ask a background process always gets: it puts the fixture in
  # the band above every ordinary window, which is the whole of what the press needs.
  [void][W]::SetWindowPos($a, [IntPtr](-1), 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010)
  Start-Sleep -Milliseconds 400
  $before = Get-Rect $a
  $cb = Get-ClientBounds $a
  $x = [int](($before.Left + $before.Right) / 2)
  # Halfway between the top of the window and the top of its client: the caption,
  # whatever height this machine's theme and scale make it.
  $y = [int](($before.Top + $cb.Top) / 2)
  if (-not (Test-IsFrontAt $a $x $y)) {
    Write-Host "  SKIPPED: the caption belongs to something else at $x,$y"
    Write-Host "   the fixture is not the window that would take a click there, so a"
    Write-Host "   drag from it would say nothing about the fixture's own frame"
  } else {
    $m = Mark
    Key-Down $VK_MENU
    try {
      if (Hold-And-Drag $x $y ($x + [int](160 * $s)) ($y + [int](120 * $s)) 12 180) {
        Check (-not (Visible (Disc))) "a drag from the caption at $x,$y opens no dial"
        Check (Said $m "was on a window's frame") "and the app says the press belonged to the frame"
        Check ((Text $m) -notmatch "wheel: opened|wheel: picked|wheel: cancelled") "and the dial never spoke"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
      Press-Up
      Start-Sleep -Milliseconds 500
    } finally { Key-Up $VK_MENU }
    $after = Get-Rect $a
    $moved = [Math]::Abs($after.Left - $before.Left) + [Math]::Abs($after.Top - $before.Top)
    Check ($moved -gt 40) "and the window came where the hand took it (moved $moved px)"
  }

  # --- 4. the watchdog ----------------------------------------------------------
  #
  # A dial left on the screen because a hand stopped moving is a screen the user
  # cannot see through and cannot click. The app's own answer is ten seconds of
  # silence; the test has to hold its breath for longer than that and generate no
  # input at all while it does.
  if (-not (Move-To $a ($v.Left + 500) ($v.Top + 300) $wide $high)) {
    Write-Host "  FAIL the fixture would not move for the watchdog step"
  } else {
    $p = Get-Rect $a
    $cb = Get-ClientBounds $a
    $x = [int](($p.Left + $p.Right) / 2)
    # The middle of the client, measured rather than guessed at from a caption
    # height: the press has to land where the dial is allowed to take it.
    $y = [int](($cb.Top + $cb.Bottom) / 2)
    $m = Mark
    Key-Down $VK_MENU
    try {
      if (Hold-And-Drag $x $y ($x + [int](110 * $s)) $y 10 180) {
        Check (Visible (Disc)) "the dial is open for the watchdog to end"
        # Nothing is sent for twelve seconds. The pointer stays where the drag put
        # it, so the only thing that can close the dial is the timer.
        Start-Sleep -Milliseconds 12500
        Check ((Text $m) -match "nothing has moved for 10s, closing") "and the app says why it closed"
        Check (-not (Visible (Disc))) "with the dial down"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      Key-Up $VK_MENU
    }
  }
} finally {
  Key-Up $VK_MENU
  Key-Up $VK_ESCAPE
  Stop-Shaiwin
  Remove-Fixtures
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
  Write-Host "cleaned up"
}
Write-Host ""
if ($script:fails -eq 0) { Write-Host "PASS" }
else { Write-Host "FAIL: $($script:fails) check(s)"; exit 1 }
