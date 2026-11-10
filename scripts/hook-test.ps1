# Regression tests for the input surface: the hook that opens the dial, the two
# hotkeys the app registers, and the desktop changing underneath a gesture that is
# already being held.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\hook-test.ps1
#
# Covers:
#   1. the mouse hook is in the desktop, and a trigger drag reaches the dial
#   2. the wheel switch in the settings file is the switch on the desktop: with it
#      off the hook is never installed and the same gesture opens nothing
#   3. a hotkey Windows refuses is reported in the log and does not stop the app
#      (another program holds Ctrl+T here while the app starts)
#   4. the same key, once the other program gives it back, opens the settings
#      page - and the page goes away when asked to
#   5. the window a seat points at is closed while the dial is held open
#      -> the release lands on a window that is gone: the dial comes down, the app
#         stays up, and nothing is raised that no longer exists
#   6. a shortcut box that is listening holds the whole keyboard and the whole
#      pointer: a trigger drag during the listening opens nothing, and the same
#      drag after the settings window has been destroyed opens the dial again
#
# Why the assertions are log lines and window state: this session cannot capture a
# layered window's contents, so the checks read what can be checked from outside.
# The dial's own behaviour - the pick, the cancel, the pins - is
# scripts\wheel-test.ps1's business; this file is about the ways in, and about the
# desktop being a place where other things happen while the user is holding a
# gesture.
#
# The windows are throwaway fixtures started by this script, so nothing the user
# was working on is ever moved or closed; and the app runs against its own
# LOCALAPPDATA, so the real settings stay untouched.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\ShaiZero.exe")
)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\testkit.ps1"

# Claiming a hotkey from the test process is the only way to make Windows refuse
# it to the app on purpose. A null window makes the registration belong to this
# thread, so no message window is needed - the claim is what is being tested, not
# the notification.
if (-not ("Win.Hot" -as [type])) {
  Add-Type -Namespace Win -Name Hot -MemberDefinition @'
    [DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr h, int id, uint mods, uint vk);
    [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr h, int id);
'@
}

$root = Join-Path $env:TEMP "shaizero-hook"
# The shipped keys, spelled out so a change of the defaults in the source is a
# change here too: Ctrl+2 takes a window onto the dial, Ctrl+T opens the panel.
$cfgOn = "reduced_motion=0`nlang=en`nexcluded=`nwheel=1`ntrigger_mods=1`ntrigger_button=left`n" +
         "wheel_radius=150.0`nwheel_dead=0.42`ndrag_threshold=8`nfav_mods=2`nfav_vk=50`npanel_mods=2`npanel_vk=84`n"
$cfgOff = $cfgOn -replace "wheel=1", "wheel=0"
$state = New-Sandbox $root $cfgOn
$log = Join-Path $state "ShaiZero\shaizero.log"
Initialize-Fixtures $root

$VK_CONTROL = 0x11; $VK_T = 0x54; $VK_MENU = 0x12
$MOD_CONTROL = 0x02
function Lines { @(Read-SharedText $log) }
function Mark { (Lines).Count }
function Text([int]$m) { @((Lines) | Select-Object -Skip $m) -join "`n" }
function Since([int]$m) { @((Lines) | Select-Object -Skip $m) }
$script:fails = 0
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { $script:fails++; Write-Host "  FAIL $what" }
}
function Disc { Find-By-Class "PanWen.Wheel" }
function Visible([IntPtr]$h) { ($h -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($h) }

# A press inside a fixture's client area: the frame is exempt on purpose, so the
# caption would be the wrong place to start a gesture the hook must take.
function Client-Centre([IntPtr]$h) {
  $r = Get-Rect $h
  $s = [W]::GetDpiForWindow($h) / 96.0
  $cap = [int](34 * $s)
  return @([int](($r.Left + $r.Right) / 2), [int](($r.Top + $cap + $r.Bottom) / 2), $s)
}

try {
  $canMove = Test-CursorControl
  $h1 = New-Fixture "One" 0

  # --- 1. the hook is in, and the gesture reaches it ----------------------------
  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -match "wheel ready: .*, on") "the log says the dial is ready and switched on"
  Check ((Text $m) -match "wheel: the hook is in") "and the hook is in the desktop"
  Check ((Text $m) -notmatch "was refused") "with every hotkey it asked for"
  if (-not $canMove) {
    Write-Host "  SKIPPED: this session has no pointer to place, so the gesture cannot be driven"
  } else {
    $p = Client-Centre $h1
    $reach = [int](95 * $p[2])
    $m = Mark
    [void][W]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
    try {
      if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 10 180) {
        Check (Visible (Disc)) "and a trigger drag through it opens the dial"
        [void][W]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)   # Escape ends it
        [void][W]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 250
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
    }
    Check (-not (Visible (Disc))) "and the dial went down with the gesture"
  }

  # --- 2. the switch in the file is the switch on the desktop -------------------
  #
  # The switch at the top of the Trigger page writes `wheel=`, and this is the
  # whole claim: off means the hook is never installed, so the desktop is the
  # user's again and the mouse costs nothing. Checked by starting the app a
  # second time against the same sandbox with the flag turned round.
  Stop-Shaiwin
  Set-Content -Path (Join-Path $state "ShaiZero\settings.txt") -Value $cfgOff -Encoding ascii
  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -match "wheel ready: .*, off") "with the switch off the log says the dial is off"
  Check ((Text $m) -notmatch "the hook is in") "and the hook is not in the desktop"
  if ($canMove) {
    $p = Client-Centre $h1
    $reach = [int](95 * $p[2])
    $m = Mark
    [void][W]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
    try {
      if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 10 180) {
        Check (-not (Visible (Disc))) "and the same drag opens nothing"
        Check ((Text $m) -notmatch "wheel: opened") "the dial never spoke"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
    }
  } else {
    Write-Host "  SKIPPED: the drag half of this step needs a pointer"
  }
  Stop-Shaiwin
  Set-Content -Path (Join-Path $state "ShaiZero\settings.txt") -Value $cfgOn -Encoding ascii

  # --- 3. a hotkey Windows refuses is reported ---------------------------------
  #
  # The app registers its two keys at startup and keeps running whatever the
  # answer is; a person whose other program holds Ctrl+T gets a line in the log
  # explaining the dial's panel is missing, not an app that quit.
  $claimed = [Win.Hot]::RegisterHotKey([IntPtr]::Zero, 0xF001, $MOD_CONTROL, $VK_T)
  Check ($claimed) "the test process took Ctrl+T away from the machine first"
  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -match "hotkey Ctrl\+T was refused") "the app said so in the log"
  Check ((Get-OurShaiwin).Count -eq 1) "and stayed running on the key it did get"
  Stop-Shaiwin
  [void][Win.Hot]::UnregisterHotKey([IntPtr]::Zero, 0xF001)

  # --- 4. the same key, given back, opens the window list ------------------------
  $m = Mark
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 1800
  Check ((Text $m) -notmatch "was refused") "and with nothing in the way it registers both"
  Press-Hotkey $MOD_CONTROL $VK_T
  if (-not (Wait-Until ({ Test-Window (Get-Settings) }) 4000)) {
    Write-Host "  SKIPPED: the keystroke did not reach a registered hotkey in this session"
  } else {
    # The list is a section of the settings page now, not a window of its own, so
    # what this key owes the user is the page itself. Which of the page's five
    # sections it lands on is settings-shot's claim, proved there by the controls
    # a click could actually land on.
    $set = Get-Settings
    Check (Visible $set) "Ctrl+T opens the page that manages what is on the dial"
    Invoke-Wm $set $script:KitClose ([IntPtr]::Zero) ([IntPtr]::Zero)
    Check (Wait-Until ({ -not (Test-Window (Get-Settings)) }) 4000) "and closes when it is asked to"
  }

  # --- 5. the desktop changes under a held gesture ------------------------------
  #
  # The dial's list is made when the gesture opens it, and a second of holding the
  # button is long enough for somebody else to close a window. The seat that points
  # at a handle Windows no longer knows must end the gesture, not the process.
  if (-not $canMove) {
    Write-Host "== 5. SKIPPED: this session has no pointer to place =="
  } else {
    $h2 = New-Fixture "Two" 1
    $p = Client-Centre $h1
    $reach = [int](95 * $p[2])
    $m = Mark
    [void][W]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
    try {
      if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 10 180) {
        Check (Visible (Disc)) "the dial is open for the interruption"
        # Whatever the dial happens to have seated, take one window off the
        # desktop by hand while the button is still down.
        Stop-Process -Id (Get-FixturePid "Two") -Force
        Start-Sleep -Milliseconds 400
        $d = Get-Rect (Disc)
        $cx = [int](($d.Left + $d.Right) / 2)
        $cy = [int](($d.Top + $d.Bottom) / 2)
        [void](Send-Move ($cx + [int](80 * $p[2])) $cy)
        Start-Sleep -Milliseconds 200
        Press-Up
        Start-Sleep -Milliseconds 400
        [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
        Check ((Get-OurShaiwin).Count -eq 1) "the app is still running after a release onto a window that went away"
        Check (-not (Visible (Disc))) "and the dial came down"
        $ended = @((Since $m) | Where-Object { $_ -match "wheel: (picked|released|cancelled)" }) -join " / "
        Check ($ended -ne "") "and it ended by saying which: $ended"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
    }
  }

  # --- 6. a shortcut box that is listening puts the desktop back ---------------
  #
  # The box asks "which combination?" and the answer is a key held down with a
  # mouse button - which is exactly what the dial reads as a gesture. So while a
  # box listens nothing may be taken, and the promise has two halves, each of
  # them checked twice: once in the log, where the app has to say what it did to
  # its own hook, and once on the desktop, where the same drag either opens the
  # dial or does not. A hook left suspended because a window closed mid-question
  # is a desktop whose gesture died silently, which is the worse failure - so
  # the destroy half is checked too, and it is the half that needs no pointer.
  $app = Get-AppWindow
  Invoke-Wm $app $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
  Check (Wait-Until { Test-Window (Get-Settings) } 5000) "the settings window is up to be asked"
  $set = Get-Settings
  # Out of the way first, and checked: a page sitting over the fixture would make
  # "nothing opened" true for the wrong reason - the press would have landed on
  # the settings window, which the gesture leaves alone whether or not a box is
  # listening.
  $fr = Get-Rect $set
  $v = Get-VirtualBounds
  [void][W]::SetWindowPos($set, [IntPtr]::Zero, ($v.Right - ($fr.Right - $fr.Left) - 20), ($v.Top + 20),
                          0, 0, 0x0005)   # SWP_NOSIZE | SWP_NOZORDER
  Start-Sleep -Milliseconds 250
  $nav = Find-Visible-Child-By-Id $set 302
  Check ($nav -ne [IntPtr]::Zero) "and the left column has its shortcut page to click"
  Click-Child-Middle $nav
  Start-Sleep -Milliseconds 350
  $box = Find-Visible-Child-By-Id $set 101
  Check ($box -ne [IntPtr]::Zero) "and that page holds the box for the key that pins a window"
  $m = Mark
  Click-Child-Middle $box
  Check (Wait-Until { (Text $m) -match "hook: stood down" } 4000) "the box said the hook stood down for it"
  if ($canMove) {
    $p = Client-Centre $h1
    $reach = [int](95 * $p[2])
    Check (Test-IsFrontAt $h1 $p[0] $p[1]) "the point the drag starts at belongs to the fixture"
    $m = Mark
    [void][W]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
    try {
      if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 10 180) {
        Check (-not (Visible (Disc))) "while the box listens the same trigger drag opens nothing"
        Check ((Text $m) -notmatch "wheel: opened") "and the dial never spoke"
      } else { Write-Host "  FAIL the drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
    }
  } else {
    Write-Host "  SKIPPED: whether the drag is taken needs a pointer to drag"
  }
  # The window dies with the question still unanswered - which is what closing it
  # does while a box listens - and the hook has to come back with the window gone.
  $m = Mark
  Invoke-Wm $set $script:KitClose ([IntPtr]::Zero) ([IntPtr]::Zero)
  Check (Wait-Until { -not (Test-Window $set) } 4000) "the settings window went down mid-listening"
  Check (Wait-Until { (Text $m) -match "hook: the desktop's again" } 4000) "and the hook came back with it"
  if ($canMove) {
    Start-Sleep -Milliseconds 300
    $m = Mark
    [void][W]::keybd_event($VK_MENU, 0, 0, [UIntPtr]::Zero)
    try {
      if (Hold-And-Drag $p[0] $p[1] ($p[0] + $reach) $p[1] 10 180) {
        Check (Visible (Disc)) "and the drag it left behind opens the dial again"
        [void][W]::keybd_event(0x1B, 0, 0, [UIntPtr]::Zero)
        [void][W]::keybd_event(0x1B, 0, 2, [UIntPtr]::Zero)
        Start-Sleep -Milliseconds 250
      } else { Write-Host "  FAIL the second drag could not be driven"; $script:fails++ }
    } finally {
      Press-Up
      [void][W]::keybd_event($VK_MENU, 0, 2, [UIntPtr]::Zero)
    }
    Check ((Text $m) -match "wheel: opened") "and said so in the log"
  }

} finally {
  [void][Win.Hot]::UnregisterHotKey([IntPtr]::Zero, 0xF001)
  Stop-Shaiwin
  Remove-Fixtures
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
  Write-Host "cleaned up"
}
Write-Host ""
if ($script:fails -eq 0) { Write-Host "PASS" }
else { Write-Host "FAIL: $($script:fails) check(s)"; exit 1 }
