# Helpers shared by the PowerShell regression tests.
#
#   . "$PSScriptRoot\testkit.ps1"
#
# Everything here is about talking to the real window manager and to the app's
# automation hooks, and about producing throwaway windows to operate on. A test
# that uses these never touches the user's applications or the real
# %LOCALAPPDATA%\ShaiZero.

Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lp);
  public delegate bool EnumProc(IntPtr h, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr lp);
  [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
  // The string form of the same call, under its own name: with both spellings
  // available under one name, PowerShell has to guess which overload a given
  // argument belongs to, and an [IntPtr]::Zero answer is not a hint it can read.
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageW(IntPtr h, uint msg, IntPtr wp, string lp);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr after, string cls, string title);
  [DllImport("user32.dll")] public static extern int GetSystemMetrics(int index);
  [DllImport("user32.dll")] public static extern bool SystemParametersInfoW(uint action, uint param, out RECT rect, uint winIni);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern bool GetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT wp);
  [DllImport("user32.dll")] public static extern bool SetWindowPlacement(IntPtr h, ref WINDOWPLACEMENT wp);
  [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int index);
  [DllImport("user32.dll", EntryPoint="GetWindowLongW")] static extern int GetWindowLongW(IntPtr h, int index);
  [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  // GetTopWindow(NULL) is the documented way to ask for the window at the very
  // top of the z-order; GetWindow needs a window that is already in it.
  [DllImport("user32.dll")] public static extern IntPtr GetTopWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int attr, out int value, int size);

  // Reading a file the ordinary way grants the reader read and write sharing but
  // not delete sharing, and the app swaps its settings in by renaming a sibling
  // over it. That is refused while a reader of that kind holds the file, so a
  // test that polls the file every few milliseconds inhibits the very write it is
  // waiting for. Asking for all three sharing modes is what a bystander means to
  // do.
  [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
  public static extern IntPtr CreateFileW(string name, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr template);

  // DWM paints nothing for a cloaked window, even when its style and rectangle
  // say it is on screen. The input host's CoreWindow is the usual one, and it
  // spans the whole desktop in some sessions.
  public static bool IsCloaked(IntPtr h) {
    int v = 0;
    return DwmGetWindowAttribute(h, 14, out v, 4) == 0 && v != 0;   // DWMWA_CLOAKED
  }

  // The dial's own promise is that it only holds the top of the screen while it
  // is being used, so the tests have to be able to see who is on top.
  public static bool IsTopmost(IntPtr h) {
    long ex = (IntPtr.Size == 8)
      ? GetWindowLongPtrW(h, -20).ToInt64()          // GWL_EXSTYLE
      : (long)GetWindowLongW(h, -20);
    return (ex & 0x8) != 0;                          // WS_EX_TOPMOST
  }
  // Is `a` painted over `b`? Walking the z-order from `b` upwards is the only
  // way to ask: the style bit alone does not answer it, because clearing
  // WS_EX_TOPMOST puts a window at the head of the normal band - still above the
  // window the user just clicked into, which is the whole complaint.
  public static bool IsAbove(IntPtr a, IntPtr b) {
    IntPtr cur = GetWindow(b, 3);                    // GW_HWNDPREV = the one above
    for (int i = 0; i < 4096 && cur != IntPtr.Zero; i++) {
      if (cur == a) return true;
      cur = GetWindow(cur, 3);
    }
    return false;
  }
  public struct RECT { public int Left, Top, Right, Bottom; }
  public struct POINT { public int X, Y; }
  public struct WINDOWPLACEMENT {
    public int length, flags, showCmd;
    public POINT ptMinPosition, ptMaxPosition;
    public RECT rcNormalPosition;
  }
}
"@

# The app works in physical pixels; so must the test, or its coordinates are
# off by the scale factor. Old Windows refuse this call and just stay unaware,
# which the tests notice and work around.
[void][W]::SetThreadDpiAwarenessContext([IntPtr](-4))

# The app's automation hooks, in one place because they are the only way a test
# can ask for a window without a hand on the mouse.
# WM_APP + 15: open the settings panel.
$script:KitOpenSettings = 0x8000 + 15
# WM_APP + 20/21: the dial's own open and pick, as if the gesture had ended where
# the pointer is. The disc window answers them; the app window does not.
$script:KitWheelOpen = 0x8000 + 20
$script:KitWheelPick = 0x8000 + 21
# WM_APP + 22 belongs to nobody now: the dial's angle is fixed for a gesture,
# so the wheel no longer posts a new one back to the app window.
$script:KitHotkey = 0x0312
$script:KitClose = 0x0010
$script:KitMouseMove = 0x0200
$script:KitMouseDown = 0x0201
$script:KitMouseUp = 0x0202

function Fail([string]$message) { throw "FAIL: $message" }

function Invoke-Wm([IntPtr]$hwnd, [uint32]$msg, [IntPtr]$wp, [IntPtr]$lp) {
  [void][W]::PostMessage($hwnd, $msg, $wp, $lp)
}

function Find-By-Class([string]$want) {
  $cb = [W+EnumProc]{
    param($h, $lp)
    # Every class looked up here is one of ShaiZero's own, and only the instances
    # this script started may be answered to. One ShaiZero per session holds the
    # mutex, so the instance that would otherwise answer is the one the user is
    # working with - and a test that moves his windows and kills his app is not
    # a test.
    $tpid = [uint32]0
    [void][W]::GetWindowThreadProcessId($h, [ref]$tpid)
    if ($script:shaiwinPids -notcontains [int]$tpid) { return $true }
    $sb = New-Object System.Text.StringBuilder 256
    [void][W]::GetClassNameW($h, $sb, 256)
    if ($sb.ToString() -eq $want) { $script:found = $h; return $false }
    return $true
  }
  $script:found = [IntPtr]::Zero
  [void][W]::EnumWindows($cb, [IntPtr]::Zero)
  return $script:found
}

function Find-By-Title([string]$want) {
  $cb = [W+EnumProc]{
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][W]::GetWindowTextW($h, $sb, 256)
    if ($sb.ToString() -eq $want -and [W]::IsWindowVisible($h)) { $script:found = $h; return $false }
    return $true
  }
  $script:found = [IntPtr]::Zero
  [void][W]::EnumWindows($cb, [IntPtr]::Zero)
  return $script:found
}

# A handle is never true or false to PowerShell on its own: IntPtr.Zero is
# truthy, so every comparison against it has to be spelled out. And a handle the
# app has already destroyed is a perfectly good number - so a test that asks
# "did that window go away" has to ask Windows, not the variable.
function Test-Window([IntPtr]$h) { return ($h -ne [IntPtr]::Zero) -and [W]::IsWindow($h) }

# The app's own hidden message window. Every automation hook except the dial's
# two arrives here.
function Get-AppWindow { Find-By-Class "PanWen.App" }

# The dial's disc, which answers the open and pick hooks.
function Get-Disc { Find-By-Class "PanWen.Wheel" }
function Get-Veil { Find-By-Class "PanWen.Veil" }
function Get-Settings { Find-By-Class "PanWen.Settings" }
function Get-TagBox { Find-By-Class "PanWen.Tag" }

# The popover's field is a stock EDIT, so a test can put words in it the way the
# keyboard does - and then hand the popover the Enter it would have got from a
# real hand. `WM_SETTEXT` is the only way in from outside that does not need the
# popover to be the foreground window, which a session with no desktop cannot
# promise. WM_SETTEXT is 0x000C, WM_KEYDOWN 0x0100, VK_RETURN 0x0D - and the
# number matters: 0x0007 is WM_SETFOCUS, which the edit answers with a shrug, so
# the field stayed empty and the file was never asked to hold a word.
function Find-Child-By-Class([IntPtr]$parent, [string]$want) {
  # `FindWindowEx` here matched nothing for a stock edit field, whose registered
  # class name is "Edit" while the callers ask for "EDIT". Walking the children
  # and comparing with PowerShell's case-insensitive -eq asks the same question
  # the callers mean.
  $script:kidByClass = [IntPtr]::Zero
  $cb = [W+EnumProc]{
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][W]::GetClassNameW($h, $sb, 256)
    if ($sb.ToString() -eq $want) { $script:kidByClass = $h; return $false }
    return $true
  }
  [void][W]::EnumChildWindows($parent, $cb, [IntPtr]::Zero)
  return $script:kidByClass
}

# The children of one class that a person could actually press. A hidden control
# is a window in the file's eyes and nothing at all in the page's, so a test that
# clicked one was clicking whatever the last section left in front.
function Get-Visible-Children([IntPtr]$parent, [string]$want) {
  $script:kids = New-Object System.Collections.ArrayList
  $cb = [W+EnumProc]{
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][W]::GetClassNameW($h, $sb, 256)
    if ($sb.ToString() -eq $want -and [W]::IsWindowVisible($h)) { [void]$script:kids.Add($h) }
    return $true
  }
  [void][W]::EnumChildWindows($parent, $cb, [IntPtr]::Zero)
  return $script:kids.ToArray()
}

# A child by the number it was created with, which is the only identifier that
# survives a change of language. The words on a panel are what a pack says; these
# four numbers are what the source says.
function Find-Visible-Child-By-Id([IntPtr]$parent, [int]$id) {
  $cb = [W+EnumProc]{
    param($h, $lp)
    if ([W]::GetDlgCtrlID($h) -eq $id -and [W]::IsWindowVisible($h)) { $script:childFound = $h; return $false }
    return $true
  }
  $script:childFound = [IntPtr]::Zero
  [void][W]::EnumChildWindows($parent, $cb, [IntPtr]::Zero)
  return $script:childFound
}

# The click a control gets when a hand lands in the middle of it: the coordinates
# are the control's own, not the screen's, because that is what `lParam` carries.
function Click-Child-Middle([IntPtr]$h) {
  $r = New-Object W+RECT
  if (-not [W]::GetClientRect($h, [ref]$r)) { Fail "a control to click has no client area" }
  $lp = [IntPtr]((([int]($r.Bottom / 2) -shl 16) -bor ([int]($r.Right / 2) -band 0xFFFF)))
  Invoke-Wm $h $script:KitMouseDown ([IntPtr]::Zero) $lp
  Invoke-Wm $h $script:KitMouseUp ([IntPtr]::Zero) $lp
  Start-Sleep -Milliseconds 250
}

function Set-Child-Text([IntPtr]$child, [string]$text) {
  [void][W]::SendMessageW($child, 0x000C, [IntPtr]::Zero, $text)
}
function Press-Enter([IntPtr]$hwnd) {
  [void][W]::SendMessage($hwnd, 0x0100, [IntPtr]0x0D, [IntPtr]::Zero)
}
function Press-Escape([IntPtr]$hwnd) {
  [void][W]::SendMessage($hwnd, 0x0100, [IntPtr]0x1B, [IntPtr]::Zero)
}

function Get-Rect([IntPtr]$h) {
  $r = New-Object W+RECT
  if (-not [W]::GetWindowRect($h, [ref]$r)) { return $null }
  return $r
}

# The screen point of a window's client (0,0) - the first pixel that belongs to
# the application rather than to its frame. A test that presses a fixed distance
# inside the rectangle is guessing at the caption, and a wrong guess lands in the
# frame, which Windows keeps for moving and sizing the window and which the dial
# is written to leave alone.
function Get-ClientOrigin([IntPtr]$h) {
  $p = New-Object W+POINT
  $p.X = 0; $p.Y = 0
  if (-not [W]::ClientToScreen($h, [ref]$p)) { return $null }
  return $p
}

# The client area in screen pixels, which is the only part of a window the
# application itself is willing to be clicked in. Returned as a rectangle so a
# test can press near a corner of it without arithmetic about borders it cannot
# see - and because a DPI-unaware window under 150% scaling has a frame whose
# height in physical pixels is not the number its own metrics give.
function Get-ClientBounds([IntPtr]$h) {
  $o = Get-ClientOrigin $h
  if ($null -eq $o) { return $null }
  $c = New-Object W+RECT
  if (-not [W]::GetClientRect($h, [ref]$c)) { return $null }
  $r = New-Object W+RECT
  $r.Left = $o.X; $r.Top = $o.Y
  $r.Right = $o.X + $c.Right; $r.Bottom = $o.Y + $c.Bottom
  return $r
}

# Is this window the one that would take a click at this pixel? The rectangle
# says where a window is, not what is on top of it, and a session full of
# overlapping windows - a terminal over a fixture - is exactly where the two
# disagree. Walking the same order the app walks when it seats the dial, and
# stopping at the first visible window that holds the point, is the honest ask.
function Test-IsFrontAt([IntPtr]$h, [int]$x, [int]$y) {
  foreach ($w in Get-ZOrder) {
    $r = Get-Rect $w
    if ($null -eq $r) { continue }
    if (-not [W]::IsWindowVisible($w) -or [W]::IsCloaked($w)) { continue }
    if ($x -ge $r.Left -and $x -lt $r.Right -and $y -ge $r.Top -and $y -lt $r.Bottom) {
      return ($w -eq $h)
    }
  }
  return $false
}

# Every top-level window, the front one first, minus ShaiZero's own layers. This is
# the same walk the app makes when it snapshots the stack before a preview, so a
# test that reads it before a gesture can say afterwards whether the position the
# window came back to is the one it left. Nothing but Windows' own order decides
# what is in it.
function Get-ZOrder {
  $list = New-Object 'System.Collections.Generic.List[System.IntPtr]'
  $cb = [W+EnumProc]{
    param($h, $lp)
    $tpid = [uint32]0
    [void][W]::GetWindowThreadProcessId($h, [ref]$tpid)
    if ($script:shaiwinPids -notcontains [int]$tpid) { [void]$list.Add($h) }
    return $true
  }
  [void][W]::EnumWindows($cb, [IntPtr]::Zero)
  return @($list)
}

# A window in the words a log line can be read from: handle, class, title, and
# whether it still exists. A stack assertion that fails has to name the windows it
# compared, or the next step is a debugger.
function Window-Blurb([IntPtr]$h) {
  if ($h -eq [IntPtr]::Zero) { return "no window" }
  $cls = New-Object System.Text.StringBuilder 256
  [void][W]::GetClassNameW($h, $cls, 256)
  return "0x$($h.ToString('x')) [$($cls.ToString())] '$(Get-WindowTitle $h)'$(if (-not [W]::IsWindow($h)) { ' gone' } else { '' })"
}

# The words in a window's title bar, as the window itself still carries them.
function Get-WindowTitle([IntPtr]$h) {
  $ttl = New-Object System.Text.StringBuilder 512
  [void][W]::GetWindowTextW($h, $ttl, 512)
  return $ttl.ToString().Trim()
}

# Whether anything is drawn where this window stands: the same three questions the
# app asks before it counts a window as being in the way. A minimized window holds a
# slot in the stack and paints nothing, a hidden one does too, and a cloaked one is
# on another desktop.
function Paints([IntPtr]$h) {
  if (-not [W]::IsWindowVisible($h)) { return $false }
  if ([W]::IsIconic($h)) { return $false }
  if ([W]::IsCloaked($h)) { return $false }
  $r = Get-Rect $h
  return ($null -ne $r) -and (($r.Right - $r.Left) -gt 0) -and (($r.Bottom - $r.Top) -gt 0)
}

# The windows a lift promised to pass. `before` is the stack as it was when the dial
# opened, so the promise is measured against the neighbours the app itself saw - a
# window that raised itself during the gesture is the desktop's business, not a broken
# preview, and a test that failed on one would be a test of whatever else is running.
# Of those neighbours only the ones that paint and overlap the lifted window count:
# a minimized explorer holds a slot in the stack and hides nothing.
function Covered-Before([IntPtr]$lifted, [Object]$before) {
  $at = [Array]::IndexOf($before, $lifted)
  $r = Get-Rect $lifted
  $bad = @()
  if ($at -lt 0 -or $null -eq $r) { return $bad }
  for ($i = 0; $i -lt $at; $i++) {
    $h = $before[$i]
    if (-not [W]::IsWindow($h)) { continue }
    if ([W]::IsTopmost($h)) { continue }
    # A window the lifted one owns rides above it however the stack is arranged:
    # Windows puts the pair back that way itself. A browser's own notice over a
    # previewed page is the window as the user left it, not a cover the preview
    # promised to pass.
    if ([W]::GetWindow($h, 4) -eq $lifted) { continue }      # GW_OWNER
    if (-not (Paints $h)) { continue }
    $q = Get-Rect $h
    $overlaps = ($q.Left -lt $r.Right) -and ($r.Left -lt $q.Right) -and ($q.Top -lt $r.Bottom) -and ($r.Top -lt $q.Bottom)
    if ($overlaps -and -not [W]::IsAbove($lifted, $h)) { $bad += $h }
  }
  return $bad
}

# The list of them, with where each stands now. Whether the lifted window moved at all
# is the first question a failure like this asks, so its own place is printed beside
# theirs, and a window with an owner is called out because Windows keeps an owned
# window above whoever owns it - a lift can no more pass that than a topmost one.
function Covering-Report([IntPtr]$lifted, [Object]$bad) {
  $now = Get-ZOrder
  # GW_HWNDPREV: the window Windows itself says is immediately over the lifted one.
  # An offender that sits there is a lift that moved the preview under the front
  # window and no further; one that sits three slots up moved there after the lift,
  # and those are different bugs with different fixes.
  $prev = [W]::GetWindow($lifted, 3)
  $lines = @($bad | ForEach-Object {
      $at = [Array]::IndexOf($now, $_)
      $owned = [W]::GetWindow($_, 4)      # GW_OWNER
      $note = if ($owned -ne [IntPtr]::Zero) { ", owned by " + (Window-Blurb $owned) } else { "" }
      $place = if ($_ -eq $prev) { ", sitting directly over it" } else { "" }
      "$(Window-Blurb $_) stands at $at$note$place"
    })
  $il = [Array]::IndexOf($now, $lifted)
  $over = Window-Blurb $prev
  return "  | $(Window-Blurb $lifted) stands at $il of $($now.Count); still over it: $($lines -join '; ')" + "`n  | the window immediately over it is $over"
}

# What Windows remembers about a window: where it comes back to, and in what
# state. A minimized window keeps the rect it had before it was minimized, which
# is how a test tells "minimized where the user can find it" from "minimized
# somewhere off screen".
function Get-Placement([IntPtr]$h) {
  $wp = New-Object W+WINDOWPLACEMENT
  $wp.length = [System.Runtime.InteropServices.Marshal]::SizeOf($wp)
  if (-not [W]::GetWindowPlacement($h, [ref]$wp)) { return $null }
  return $wp
}

function Get-VirtualBounds {
  $x = [W]::GetSystemMetrics(76)   # SM_XVIRTUALSCREEN
  $y = [W]::GetSystemMetrics(77)
  $w = [W]::GetSystemMetrics(78)
  $h = [W]::GetSystemMetrics(79)
  return New-Object W+RECT -Property @{ Left = $x; Top = $y; Right = ($x + $w); Bottom = ($y + $h) }
}

function Move-Cursor([int]$x, [int]$y) {  for ($try = 1; $try -le 4; $try++) {
    if (-not [W]::SetCursorPos($x, $y)) { Start-Sleep -Milliseconds 150; continue }
    Start-Sleep -Milliseconds 120
    $p = New-Object W+POINT
    if (-not [W]::GetCursorPos([ref]$p)) { continue }
    if ($p.X -eq $x -and $p.Y -eq $y) { return $true }
    # Somebody else moved the pointer - a hand on the same machine. Say so and
    # try again rather than failing the case on the first nudge.
    Write-Host "    the pointer drifted to ($($p.X),$($p.Y)); trying again"
  }
  return $false
}

# Real mouse input, because a posted message skips the hit-test and the capture
# the window manager sets up for itself, and half of what a control does is the
# gesture that reaches it.
function Press-At([int]$x, [int]$y) {
  if (-not (Move-Cursor $x $y)) { return $false }
  Start-Sleep -Milliseconds 120   # let the pointer land before the button does
  [W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)   # LEFTDOWN
  return $true
}

function Release-Mouse {
  [W]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)   # LEFTUP
  Start-Sleep -Milliseconds 80
}

function Click-At([int]$x, [int]$y) {
  if (-not (Press-At $x $y)) { return $false }
  Release-Mouse
  return $true
}

# --- driving a gesture ---------------------------------------------------------

# `SetCursorPos` above moves the pointer, but it does not put a WM_MOUSEMOVE
# through the low-level hook, and the dial opens on a hook move. This is the same
# absolute injection the app's own click replay uses, so the app sees what a hand
# would see - and the check afterwards is that the pointer really landed, because
# a session with no input desktop accepts the call and ignores it.
#
# The desktop's bounds are read again for every move rather than once here: a test
# that turns the thread per-monitor DPI aware after this file is loaded would
# otherwise normalise physical coordinates against the unaware view of the same
# desktop, and a point that is off the end of that view wraps into a negative
# number the injection cannot even take.
function Send-Move([int]$x, [int]$y) {
  $v = Get-VirtualBounds
  if ($x -lt $v.Left -or $x -ge $v.Right -or $y -lt $v.Top -or $y -ge $v.Bottom) {
    Write-Host "  the pointer cannot be placed at $x,$y - the desktop is $($v.Left),$($v.Top) to $($v.Right),$($v.Bottom)"
    return $false
  }
  # 0x0001 move, 0x8000 absolute, 0x4000 virtual-desktop coordinates.
  $wide = [Math]::Max($v.Right - $v.Left - 1, 1)
  $high = [Math]::Max($v.Bottom - $v.Top - 1, 1)
  $px = $x - $v.Left
  $py = $y - $v.Top
  $nx = [int][Math]::Round($px * 65535.0 / $wide)
  $ny = [int][Math]::Round($py * 65535.0 / $high)
  $p = New-Object W+POINT
  # The wanted pixel first, then the two neighbours of each axis: the mapping from
  # a normalised coordinate to a pixel is not the exact inverse of the mapping the
  # test does by hand, and the difference is one pixel at the boundary between two
  # pixels - enough to make a test that asked for sixteen pixels from the corner
  # give up. What is asserted is where the cursor actually is, not what was sent.
  $step = $nx + 1
  $back = [Math]::Max(0, $nx - 1)
  $rise = $ny + 1
  $fell = [Math]::Max(0, $ny - 1)
  foreach ($d in @(@($nx, $ny), @($step, $ny), @($back, $ny), @($nx, $rise), @($nx, $fell))) {
    [W]::mouse_event(0x0001 -bor 0x8000 -bor 0x4000, $d[0], $d[1], 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 25
    if (-not [W]::GetCursorPos([ref]$p)) { return $false }
    if ($p.X -eq $x -and $p.Y -eq $y) { return $true }
  }
  Write-Host "  the pointer would not sit at $x,$y - it stopped at $($p.X),$($p.Y)"
  return $false
}

function Press-Down { [W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero) }   # LEFTDOWN
function Press-Up { [W]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80 }

# The gesture, with the button held between the two ends. The release is always
# written as its own step in the tests that use this, because what the pointer is
# doing when the button comes up is the whole difference between a pick, a cancel
# and a pin.
function Hold-And-Drag([int]$x1, [int]$y1, [int]$x2, [int]$y2, [int]$steps, [int]$pauseMs) {
  if (-not (Send-Move $x1 $y1)) { return $false }
  Start-Sleep -Milliseconds 120
  Press-Down
  Start-Sleep -Milliseconds $pauseMs
  for ($i = 1; $i -le $steps; $i++) {
    $x = [int]($x1 + ($x2 - $x1) * $i / $steps)
    $y = [int]($y1 + ($y2 - $y1) * $i / $steps)
    if (-not (Send-Move $x $y)) { Press-Up; return $false }
  }
  Start-Sleep -Milliseconds $pauseMs
  return $true
}

# A registered hotkey is global, so the honest way to press one is the way a hand
# presses it: hold the modifiers, tap the key, let go in the other order.
function Press-Hotkey([byte]$mods, [byte]$vk) {
  if ($mods -band 0x0001) { [void][W]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero) }   # ALT
  if ($mods -band 0x0002) { [void][W]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero) }   # CTRL
  if ($mods -band 0x0004) { [void][W]::keybd_event(0x10, 0, 0, [UIntPtr]::Zero) }   # SHIFT
  if ($mods -band 0x0008) { [void][W]::keybd_event(0x5B, 0, 0, [UIntPtr]::Zero) }   # WIN
  Start-Sleep -Milliseconds 60
  [void][W]::keybd_event($vk, 0, 0, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 60
  [void][W]::keybd_event($vk, 0, 2, [UIntPtr]::Zero)
  Start-Sleep -Milliseconds 60
  if ($mods -band 0x0008) { [void][W]::keybd_event(0x5B, 0, 2, [UIntPtr]::Zero) }
  if ($mods -band 0x0004) { [void][W]::keybd_event(0x10, 0, 2, [UIntPtr]::Zero) }
  if ($mods -band 0x0002) { [void][W]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
  if ($mods -band 0x0001) { [void][W]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero) }
}

# The lock screen is a full-screen window in front of everything and it takes
# every click, but moving the pointer behind it still succeeds. A test that
# cannot get past it has to skip its pointer steps instead of failing them.
function Test-LockScreen {
  $fg = [W]::GetForegroundWindow()
  if ($fg -eq [IntPtr]::Zero) { return $false }
  $fgPid = [uint32]0
  [void][W]::GetWindowThreadProcessId($fg, [ref]$fgPid)
  return (Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName -eq "LockApp"
}

# A session with no input desktop (a service, or an agent shell) cannot move or
# even read the cursor. Anything that depends on the pointer has to be skipped
# rather than failed there. The pointer is put back where it was found.
function Test-CursorControl {
  if (Test-LockScreen) {
    Write-Host "  the lock screen is up; clicks cannot reach anything"
    return $false
  }
  $p = New-Object W+POINT
  [void][W]::GetCursorPos([ref]$p)
  $mid = New-Object W+POINT
  $mid.X = [int]([W]::GetSystemMetrics(0) / 2)
  $mid.Y = [int]([W]::GetSystemMetrics(1) / 2)
  $ok = Move-Cursor $mid.X $mid.Y
  if ($ok) { [void][W]::SetCursorPos($p.X, $p.Y) }
  return $ok
}

# Visible and on a monitor, as opposed to sitting in the taskbar.
function Test-OnDesktop([IntPtr]$h) {
  return ([W]::IsWindowVisible($h) -and -not [W]::IsIconic($h))
}

function Wait-Until([scriptblock]$condition, [int]$timeoutMs) {
  $end = [Environment]::TickCount + $timeoutMs
  while ([Environment]::TickCount -lt $end) {
    if ([bool](& $condition)) { return $true }
    Start-Sleep -Milliseconds 60
  }
  return [bool](& $condition)
}

function Wait-Visibility([IntPtr]$h, [bool]$want, [int]$timeoutMs) {
  $end = [Environment]::TickCount + $timeoutMs
  while ([Environment]::TickCount -lt $end) {
    if ([W]::IsWindowVisible($h) -eq $want) { return $true }
    Start-Sleep -Milliseconds 50
  }
  return ([W]::IsWindowVisible($h) -eq $want)
}

# --- reading the files the app writes ------------------------------------------

# One reader for everything the tests read that the app also writes - its settings
# above all. `Get-Content` is not it: .NET grants a reader read and write sharing
# but never delete sharing, and the settings file is swapped in by renaming a
# sibling over it. That is refused for as long as such a reader holds the file, so
# a test that polls it every few milliseconds inhibits the very write it is
# waiting for - it then sees an old file that is only still there because of the
# test. CreateFile with all three sharing modes is what a bystander means to do.
function Assert-NoCut([string]$state, [string]$where) {
  # The settings panel measures every line before it draws and logs the ones it
  # has to cut, so a translation that overflows is a failing test rather than a
  # sentence the reader has to guess the end of.
  $log = @(Read-SharedText (Join-Path $state "ShaiZero\shaizero.log"))
  $cut = @($log | Where-Object { $_ -match "has no room" })
  if ($cut.Count -gt 0) { Fail "$where cut a line:`n$($cut -join "`n")" }
  Write-Host "  nothing was cut $where"
}

function Read-SharedText([string]$path) {
  if (-not (Test-Path $path)) { return @() }
  # 1 = FILE_READ_DATA, 7 = read|write|delete sharing, 3 = OPEN_EXISTING,
  # 0x80 = FILE_ATTRIBUTE_NORMAL. Not GENERIC_READ: PowerShell 5.1 turns that
  # literal into a negative number and refuses to hand it to a uint32 parameter.
  $last = 0
  for ($try = 1; $try -le 5; $try++) {
    $h = [W]::CreateFileW($path, 1, 7, [IntPtr]::Zero, 3, 0x80, [IntPtr]::Zero)
    if ($h -ne [IntPtr](-1)) {
      $sh = New-Object Microsoft.Win32.SafeHandles.SafeFileHandle($h, $true)
      $fs = New-Object System.IO.FileStream($sh, [System.IO.FileAccess]::Read)
      $reader = New-Object System.IO.StreamReader($fs)
      $text = $reader.ReadToEnd()
      $reader.Dispose()
      # A log is a list of lines, and the newline at the end of the last one is
      # part of that line, not the start of another: `-split` says otherwise and
      # hands back an empty string for the position after the final newline. Every
      # caller here counts lines to mark where it was in the file and then asks for
      # what came after, so a phantom line makes the mark one too high and the
      # first line written after it disappears - which is exactly the line a step
      # is looking for. `Get-Content` does not have this problem, and neither may
      # this.
      $parts = @($text -split "`r?`n")
      if ($parts.Count -gt 1 -and $parts[-1] -eq "") { $parts = @($parts[0..($parts.Count - 2)]) }
      elseif ($parts.Count -eq 1 -and $parts[0] -eq "") { $parts = @() }
      return $parts
    }
    $last = [System.Runtime.InteropServices.Marshal]::GetLastWin32Error()
    # The file is being swapped in this instant. Back off and ask again; a reader
    # that gives up at once would report a file that is there as one that is
    # not, and a test cannot be trusted to notice the difference.
    Start-Sleep -Milliseconds 40
  }
  if (-not (Test-Path $path)) { return @() }
  Fail "$path is there and this script cannot read it (Win32 error $last)"
  return @()
}

# --- throwaway windows ---------------------------------------------------------

$script:FixtureSource = @'
param([string]$Title, [int]$X, [int]$Y)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
# The console is this fixture's shell, not part of the test. Hiding the console
# window from inside is what works: starting the process with -WindowStyle
# Hidden also hides the form, because a new top-level window takes its show
# state from the process.
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class Con {
  [DllImport("kernel32.dll")] public static extern IntPtr GetConsoleWindow();
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int n);
}
"@
[void][Con]::ShowWindow([Con]::GetConsoleWindow(), 0)
$form = New-Object System.Windows.Forms.Form
$form.Text = $Title
$form.StartPosition = 'Manual'
$form.Location = New-Object System.Drawing.Point($X, $Y)
$form.Size = New-Object System.Drawing.Size(460, 320)
[System.Windows.Forms.Application]::Run($form)
'@

$script:fixturePids = @()
$script:fixtureHandles = @()
$script:fixtureDir = ""

function Initialize-Fixtures([string]$dir) {
  $script:fixtureDir = $dir
  New-Item -ItemType Directory -Force -Path $dir | Out-Null
  Set-Content -Path (Join-Path $dir "fixture.ps1") -Value $script:FixtureSource -Encoding ascii
}

function New-Fixture([string]$name, [int]$index) {
  $title = "ShaiWinFixture$name"
  $v = Get-VirtualBounds
  $x = $v.Left + 120 + $index * 90
  $y = $v.Top + 200 + $index * 70
  $shell = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
  $childArgs = @("-NoProfile", "-ExecutionPolicy", "Bypass", "-STA", "-File", (Join-Path $script:fixtureDir "fixture.ps1"), "-Title", $title, "-X", $x, "-Y", $y)
  $proc = Start-Process $shell -ArgumentList $childArgs -PassThru
  $script:fixturePids += @{ Title = $title; Id = $proc.Id }
  if (-not (Wait-Until ({ (Find-By-Title $title) -ne [IntPtr]::Zero }) 20000)) {
    Fail "fixture $title never showed a window (pid $($proc.Id))"
  }
  $h = Find-By-Title $title
  $script:fixtureHandles += $h
  return $h
}

function Get-FixturePid([string]$name) {
  $title = "ShaiWinFixture$name"
  ($script:fixturePids | Where-Object { $_.Title -eq $title } | Select-Object -First 1).Id
}

function Remove-Fixtures {
  foreach ($h in $script:fixtureHandles) {
    if ($h -ne [IntPtr]::Zero) { Invoke-Wm $h $script:KitClose ([IntPtr]::Zero) ([IntPtr]::Zero) }
  }
  Start-Sleep -Milliseconds 400
  foreach ($owned in $script:fixturePids) {
    Stop-Process -Id $owned.Id -Force -ErrorAction SilentlyContinue
  }
}

# --- the app itself ------------------------------------------------------------

# The instances this script started. Everything the tests drive - the window they
# send messages to, the process they wait for or stop - comes out of this list.
# Only one ShaiZero can hold the session mutex, so the instance that would
# otherwise answer is the user's own, and a test that switches his windows about
# and kills his app is not a test, it is a support call.
$script:shaiwinPids = @()

function Get-OurShaiwin {
  if (-not $script:shaiwinPids) { return @() }
  @(Get-Process -Id $script:shaiwinPids -ErrorAction SilentlyContinue)
}

# Refuse to run alongside an instance this script did not start. There is no
# safe way to tell the leftover of an aborted run from an instance someone is
# working with, so the human decides.
function Assert-NoForeignShaiwin {
  $foreign = @(Get-Process ShaiZero -ErrorAction SilentlyContinue |
    Where-Object { $script:shaiwinPids -notcontains $_.Id })
  if ($foreign) {
    Fail ("a ShaiZero this script did not start is running (pid $(($foreign | ForEach-Object { $_.Id }) -join ', ')). " +
          "Quit it from the tray and run this again; if it is the leftover of an aborted run, stop it yourself. " +
          "The tests will not stop it for you - it may be the instance you are working with.")
  }
}

# Stop the instances this script started, after checking that no instance of the
# user's is running to be confused with them. Whatever the dial wrote as it went -
# a pin, a favourite - is already in the file, because the app writes the moment
# one of them changes rather than on the way out.
function Stop-Shaiwin {
  Assert-NoForeignShaiwin
  if (-not $script:shaiwinPids) { return }
  foreach ($owned in $script:shaiwinPids) {
    Stop-Process -Id $owned -Force -ErrorAction SilentlyContinue
  }
  $script:shaiwinPids = @()
  Start-Sleep -Milliseconds 500
}

# Hard-stop the instances this script started and leave their windows exactly
# where they stand - the way a crash leaves them, which is what the tests about
# coming back want.
function Stop-OurShaiwin {
  foreach ($owned in $script:shaiwinPids) {
    Stop-Process -Id $owned -Force -ErrorAction SilentlyContinue
  }
  $script:shaiwinPids = @()
  Start-Sleep -Milliseconds 600
}

# A private LOCALAPPDATA so the tests can never write the real settings file. The caller keeps whatever was there.
function New-Sandbox([string]$root, [string]$settings) {
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
  $state = Join-Path $root "localappdata"
  New-Item -ItemType Directory -Force (Join-Path $state "ShaiZero") | Out-Null
  Set-Content -Path (Join-Path $state "ShaiZero\settings.txt") -Value $settings -Encoding ascii
  return $state
}

function Start-Shaiwin([string]$exe, [string]$state) {
  Assert-NoForeignShaiwin
  $saved = $env:LOCALAPPDATA
  $env:LOCALAPPDATA = $state
  try {
    $proc = Start-Process -FilePath $exe -PassThru
    $script:shaiwinPids += $proc.Id
    return $proc
  } finally { $env:LOCALAPPDATA = $saved }
}
