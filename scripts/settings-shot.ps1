# Look at the settings window without anybody having to click it - and click the
# things on it, so that "the panel writes the file" is a tested claim and not a
# hope.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\settings-shot.ps1
#
# Starts the app against a private LOCALAPPDATA, asks it to open its settings
# (WM_APP+15 on the app window), captures the window with PrintWindow, then posts
# messages at the shortcut boxes, the switch, the trigger cells and the steppers: a
# box clicked is listening, a box whose x was clicked is empty, and the row that
# moved writes the settings file itself - there is no Save button left to press.
# Every check here is therefore "changed it, then read the file", not "changed it
# and pressed something".
#
# The page shows one section at a time, named by the column on the left, so the
# first thing every block does is click that column. An enumeration here is of the
# controls a person could press, which is also how "the other sections are not on
# this page" gets tested rather than assumed. The last two sections are the two
# things a screenshot cannot say: that the frame is as tall as its tallest section
# on this screen, and that a row of the window list answers a click on the spot.
#
# The PNGs land in %TEMP%\shaizero-settings-shots.
#
# Like the other tests this refuses to run alongside an instance the script did
# not start, so quit the tray icon first - and the real settings are never read or
# written.
param(
  [string]$Exe = (Join-Path $PSScriptRoot "..\target\release\ShaiZero.exe")
)

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\testkit.ps1"

Add-Type -AssemblyName System.Drawing
if (-not ("Win.Print" -as [type])) {
  Add-Type -Namespace Win -Name Print -MemberDefinition @'
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassNameW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
'@
}

$shotDir = Join-Path $env:TEMP "shaizero-settings-shots"
New-Item -ItemType Directory -Force -Path $shotDir | Out-Null
Get-ChildItem $shotDir -Filter "*.png" -ErrorAction SilentlyContinue | Remove-Item -Force

$script:Down = [uint32]0x0201
$script:Up = [uint32]0x0202
$script:Move = [uint32]0x0200
$script:KeyDown = [uint32]0x0100
$script:VKEscape = [IntPtr]0x1B
# The two ends of the two steppers, spelled from code points: this file has no
# byte-order mark, so Windows PowerShell reads it in the ANSI code page and a
# minus sign written as a literal arrives as something else.
$script:Minus = [string][char]0x2212
$script:Plus = "+"
# The English caption of the door to the language files, which ends in an
# ellipsis character for the same reason the minus sign does.
$script:EnFolder = "Language files$([char]0x2026)"
# The five names down the left, by the id the panel numbers them with. Not by the
# word on them: a page that speaks French calls its own first row something else,
# and the column is the one thing in this window that is in the same order in
# every language.
$script:NavId = @{ Trigger = 301; Shortcuts = 302; Windows = 303; Language = 304; About = 305 }

# Every control of every section exists the moment the window opens - a section is
# shown and hidden, never rebuilt, because the chain that applies a row the instant
# it moves has to stay the one thing nobody has to re-make. So an enumeration here
# asks Windows what is actually on the page: a handle to a hidden control is not
# something a person could press, and a test that pressed one would be testing the
# message queue rather than the panel.
function Get-Children([IntPtr]$parent, [string]$want) {
  $script:kids = New-Object System.Collections.ArrayList
  $cb = [Win.Print+EnumProc]{
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][Win.Print]::GetClassNameW($h, $sb, 256)
    if ($sb.ToString() -eq $want -and [W]::IsWindowVisible($h)) { [void]$script:kids.Add($h) }
    return $true
  }
  [void][Win.Print]::EnumChildWindows($parent, $cb, [IntPtr]::Zero)
  return $script:kids.ToArray()
}

function Get-Box([IntPtr]$h) {
  $r = Get-Rect $h
  if (-not $r) { Fail "no rect for a child window" }
  # One subtraction per line: PowerShell binds the comma tighter than the minus,
  # so @($r.Right - $r.Left, $r.Bottom - $r.Top) asks an array to subtract.
  $w = $r.Right - $r.Left
  $t = $r.Bottom - $r.Top
  if ($w -lt 2 -or $t -lt 2) { Fail "the control has no size ($w x $t)" }
  return @{ w = $w; t = $t }
}

function Get-Text([IntPtr]$h) {
  $sb = New-Object System.Text.StringBuilder 512
  [void][Win.Print]::GetWindowTextW($h, $sb, 512)
  return $sb.ToString()
}

# The buttons are found by the word on them, never by index: which language the
# panel speaks decides how many there are and what they say, and a test that
# counted them would be testing its own arithmetic. The steppers' two ends are
# words too, as far as this file is concerned - a minus is a minus in every
# language, and there are exactly two of them because there are two numbers to
# step.
function Get-Button-ByText([IntPtr]$parent, [string]$want) {
  @(Get-Children $parent "PanWen.Settings.Button" | Where-Object { (Get-Text $_) -eq $want })
}

function Click-At([IntPtr]$h, [int]$x, [int]$y) {
  # lParam is the position in the control's own client coordinates.
  $lp = Pack-Lp $x $y
  Invoke-Wm $h $script:Down ([IntPtr]::Zero) $lp
  Invoke-Wm $h $script:Up ([IntPtr]::Zero) $lp
  Start-Sleep -Milliseconds 250
}

function Click-Middle([IntPtr]$h) {
  $box = Get-Box $h
  $x = [int]($box.w / 2)
  $y = [int]($box.t / 2)
  Click-At $h $x $y
}

function Get-Button-ById([IntPtr]$parent, [int]$id) {
  @(Get-Children $parent "PanWen.Settings.Button" |
      Where-Object { [Win.Print]::GetDlgCtrlID($_) -eq $id })
}

# The click that moves the page from one section to the next. The four names down
# the left are the only way a person gets from the shortcuts to the languages, so
# every block that needs a section asks for it here first - and the checks that a
# section's controls are the only ones on the page are the other half of that
# claim.
function Goto-Section([IntPtr]$parent, [int]$id) {
  $button = @(Get-Button-ById $parent $id)
  if ($button.Count -ne 1) { Fail "the left column does not have one button numbered $id ($($button.Count) of them)" }
  Click-Middle $button[0]
  Start-Sleep -Milliseconds 350
}

function Pack-Lp([int]$x, [int]$y) {
  return [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
}

# EnumChildWindows walks the z-order, which is not the order the controls were
# made in, so a test must never read this list by index. Sorting by the rect puts
# the rows back top to bottom, and the two steppers are laid out one under the
# other - so the first plus is the wheel's size and the second is the distance.
function Get-Children-Sorted([IntPtr]$parent, [string]$want) {
  @(Get-Children $parent $want | Sort-Object { (Get-Rect $_).Top })
}

function Save-Shot([IntPtr]$hwnd, [string]$path) {
  $r = New-Object W+RECT
  if (-not [W]::GetWindowRect($hwnd, [ref]$r)) { Fail "no rect for $path" }
  $w = $r.Right - $r.Left
  $h = $r.Bottom - $r.Top
  if ($w -lt 20 -or $h -lt 20) { Fail "the window is not really there ($w x $h)" }
  # PrintWindow, not the screen: this shell has no desktop to grab. Every window
  # in the panel paints itself for exactly that reason - an owner-drawn stock
  # button answers WM_DRAWITEM instead and comes back as a blank grey rectangle.
  $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $gfx = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $gfx.GetHdc()
  $ok = [Win.Print]::PrintWindow($hwnd, $hdc, 2)   # PW_RENDERFULLCONTENT
  $gfx.ReleaseHdc($hdc)
  $gfx.Dispose()
  if (-not $ok) { $bmp.Dispose(); Fail "PrintWindow failed for $path" }
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Host "shot: $path ($w x $h)"
}

function Open-Settings([string]$name, [string]$settings) {
  $root = Join-Path $env:TEMP "shaizero-settings-$name"
  $state = New-Sandbox $root $settings
  $proc = Start-Shaiwin $Exe $state
  Start-Sleep -Milliseconds 2200
  $app = Get-AppWindow
  if (-not (Test-Window $app)) { Fail "the app window is not there" }
  Invoke-Wm $app $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
  if (-not (Wait-Until { Test-Window (Get-Settings) } 5000)) {
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Fail "the settings window never opened"
  }
  Start-Sleep -Milliseconds 900
  return @{
    state = $state
    proc = $proc
    wnd = Get-Settings
    scale = [W]::GetDpiForWindow((Get-Settings)) / 96.0
  }
}

function Close-Settings($session) {
  if ($session) {
    Stop-Process -Id $session.proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
  }
}

function Saved-Text($session) {
  return @((Read-SharedText (Join-Path $session.state "ShaiZero\settings.txt"))) -join "`n"
}

# A row applies itself the moment it moves, so the wait is for the file, not for
# a click on a button that no longer exists. Waiting for the line rather than
# sleeping a fixed time keeps the claim what it is: the write happened, and this
# is where it turned up.
function Wait-FileLine($session, [string]$pattern) {
  return Wait-Until { (Saved-Text $session) -match $pattern } 4000
}

# The shipped screen: the wheel on, its trigger Alt + left button, the dial 150
# logical pixels wide, a gesture starting after eight pixels, and the two keys the
# program comes with.
$base = "reduced_motion=0`nlang=en`nexcluded=`nwheel=1`ntrigger_mods=1`ntrigger_button=left`n" +
        "wheel_radius=150.0`nwheel_dead=0.42`ndrag_threshold=8`nfav_mods=2`nfav_vk=50`npanel_mods=2`npanel_vk=84`n"

$script:fails = 0
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { $script:fails++; Write-Host "  FAIL $what" }
}
function Visible([IntPtr]$h) { ($h -ne [IntPtr]::Zero) -and [W]::IsWindowVisible($h) }

Write-Host "== the window, as it opens =="
Stop-Shaiwin

# Alt+left is the wheel's trigger, Ctrl+2 the shortcut that takes a window onto
# the dial, Ctrl+T the window list.
$session = Open-Settings "shipped" $base
try {
  Save-Shot $session.wnd (Join-Path $shotDir "settings-shipped.png")
  Assert-NoCut $session.state "in the language it ships"

  Write-Host "== what is on the page, one section at a time =="
  # The left column names five sections and the right one holds the section that
  # was clicked. Every count below is of the windows a click could land on, which
  # is how the page shows one section rather than five stacked under each other.
  # `the_window_is_as_tall_as_its_tallest_section` in
  # crates/app/src/settings_ui.rs is the same claim about the arithmetic; this is
  # the claim about the pixels of this screen.
  $frame = Get-Rect $session.wnd
  $v = Get-VirtualBounds
  # It opens on the dial's own rows, which is what a person came here to change.
  Check ((Get-Button-ByText $session.wnd "Shift").Count -eq 1) "the page opens on the dial's rows"
  Check ((Get-Button-ByText $session.wnd "Pin").Count -eq 0) "and not on the window list"

  # `boxes`, `sw` and `step` rather than `keys`, `switches` and `steps`: a
  # hashtable answers `.keys` with its own key collection, and a test that read
  # that would be comparing a number against a list.
  # The windows and shortcuts pages have no word of their own to look for: one is
  # two boxes that capture whatever key is pressed, and the other says words that
  # depend on how many windows the desktop is running. Both are checked for what
  # they hold further down, and the negative half below is still the whole claim
  # that a section shows only itself.
  $sections = @(
    @{ name = "Trigger"; boxes = 0; sw = 1; step = 4
       on = @("Shift", "left button") },
    @{ name = "Shortcuts"; boxes = 2; sw = 0; step = 0
       on = @() },
    @{ name = "Windows"; boxes = 0; sw = 0; step = 0
       on = @() },
    @{ name = "Language"; boxes = 0; sw = 0; step = 0
       on = @($script:EnFolder, "English") },
    @{ name = "About"; boxes = 0; sw = 1; step = 0
       on = @("Open the log") }
  )
  foreach ($want in $sections) {
    Goto-Section $session.wnd $script:NavId[$want.name]
    $keys = @(Get-Children-Sorted $session.wnd "PanWen.Settings.Key")
    Check ($keys.Count -eq $want.boxes) "$($want.name): $($want.boxes) shortcut box(es) on the page (found $($keys.Count))"
    $switches = @(Get-Children $session.wnd "PanWen.Settings.Switch")
    Check ($switches.Count -eq $want.sw) "$($want.name): $($want.sw) switch(es) on the page (found $($switches.Count))"
    # Each side is wrapped before it is added: a function that finds nothing
    # hands back nothing, and PowerShell reads that as `$null` - and `$null`
    # plus `$null` is the number zero, which would look like a stepper.
    $ends = @(@(Get-Button-ByText $session.wnd $script:Plus) +
              @(Get-Button-ByText $session.wnd $script:Minus))
    Check ($ends.Count -eq $want.step) "$($want.name): $($want.step) stepper ends on the page (found $($ends.Count))"
    foreach ($text in $want.on) {
      Check ((Get-Button-ByText $session.wnd $text).Count -eq 1) "$($want.name): '$text' is on the page"
    }
    # The other sections' words are the negative half of the same claim: a control
    # that belongs to another page is not on this one.
    foreach ($other in $sections) {
      if ($other.name -eq $want.name) { continue }
      foreach ($text in $other.on) {
        Check ((Get-Button-ByText $session.wnd $text).Count -eq 0) "$($want.name): '$text' belongs to $($other.name), not to this page"
      }
    }
    # The names themselves never leave, in any section.
    foreach ($label in "Trigger", "Shortcuts", "Windows", "Language", "About") {
      Check ((Get-Button-ByText $session.wnd $label).Count -eq 1) "$($want.name): the name '$label' is still in the left column"
    }
    # Clicking a name changes what is on the page and nothing else - not the frame
    # the pointer was aimed at, and not the height the window was opened at.
    $now = Get-Rect $session.wnd
    Check (($now.Left -eq $frame.Left) -and ($now.Top -eq $frame.Top) -and
           ($now.Right -eq $frame.Right) -and ($now.Bottom -eq $frame.Bottom)) "$($want.name): the frame did not move"
    # The lowest thing on the page is a control, not a card: a row below the work
    # area is a setting nobody can reach, because this page has no scrollbar.
    $low = 0
    foreach ($h in @(Get-Children $session.wnd "PanWen.Settings.Button") +
                   @(Get-Children $session.wnd "PanWen.Settings.Key") +
                   @(Get-Children $session.wnd "PanWen.Settings.Switch")) {
      $r = Get-Rect $h
      if ($r.Bottom -gt $low) { $low = $r.Bottom }
    }
    Check ($low -gt 0 -and $low -le $v.Bottom) "$($want.name): the last control is on the screen ($low of $($v.Bottom))"
    Save-Shot $session.wnd (Join-Path $shotDir "settings-$($want.name.ToLower()).png")
  }
  # A section a person cannot see is a section whose words cannot be measured for
  # cutting, so this is the sentence that says all five pages have room for what
  # they say.
  Assert-NoCut $session.state "in all five sections"
  # The two buttons that used to sit at the bottom of this window are the promise
  # of the whole design: a row that applies itself needs nobody to confirm it, and
  # a row nobody confirmed cannot be undone by a button either.
  Check ((Get-Button-ByText $session.wnd "Save").Count -eq 0) "there is no Save left to press"
  Check ((Get-Button-ByText $session.wnd "Cancel").Count -eq 0) "and no Cancel to undo a row with"

  Write-Host "== the shortcut box, clicked =="
  Goto-Section $session.wnd $script:NavId.Shortcuts
  $keys = @(Get-Children-Sorted $session.wnd "PanWen.Settings.Key")
  if ($keys.Count -ne 2) { Fail "the shortcuts page has $($keys.Count) boxes, not two" }
  Click-Middle $keys[0]
  Save-Shot $session.wnd (Join-Path $shotDir "settings-listening.png")
  # Escape puts the box back the way it found it.
  Invoke-Wm $keys[0] $script:KeyDown $script:VKEscape ([IntPtr]::Zero)
  Start-Sleep -Milliseconds 400
  # Listening is not deciding, so the file is still the one this sandbox was
  # seeded with. The three list lines are the ones only a save writes: a window
  # that had applied anything would have laid the whole file out again.
  $untouched = -not ((Saved-Text $session) -match "favorite_slots")
  Check $untouched "a box that was only clicked, then escaped, wrote nothing at all"

  Write-Host "== the x clears a shortcut, and the row writes it =="
  $box = Get-Box $keys[1]
  # The x lives in the right-hand end of the field, 26 logical pixels wide.
  Click-At $keys[1] ($box.w - [int](13 * $session.scale)) ([int]($box.t / 2))
  Save-Shot $session.wnd (Join-Path $shotDir "settings-cleared.png")
  # Nobody pressed anything. The click on the x is the whole decision.
  Check (Wait-FileLine $session "panel_vk=0") "the cleared shortcut is in the file on its own: Ctrl+T is off"
  $saved = Saved-Text $session
  Check ($saved -match "fav_vk=50") "and the untouched shortcut moved nowhere: Ctrl+2 still works"
  Check ($saved -match "wheel=1") "and the switch nobody touched is still on"
} finally {
  Close-Settings $session
}

Write-Host "== an unset shortcut, before anybody touches it =="
Stop-Shaiwin
$session = Open-Settings "unset" ($base + "fav_vk=0`npanel_vk=0`n")
try {
  Goto-Section $session.wnd $script:NavId.Shortcuts
  Save-Shot $session.wnd (Join-Path $shotDir "settings-unset.png")
} finally {
  Close-Settings $session
}

Write-Host "== the switch, the trigger and the two steppers, operated =="
Stop-Shaiwin
$session = Open-Settings "controls" $base
try {
  Save-Shot $session.wnd (Join-Path $shotDir "settings-before.png")
  # The dial's rows are what the window opens on, and everything clicked below is
  # on that page - so the click that says so is made here rather than assumed.
  Goto-Section $session.wnd $script:NavId.Trigger

  $switches = @(Get-Children $session.wnd "PanWen.Settings.Switch" | Sort-Object { (Get-Rect $_).Top })
  Click-Middle $switches[0]
  # The switch is the wheel itself, so it goes back on before anything else is
  # touched: the rest of this section is about the numbers, not about whether the
  # hook is installed at all (scripts\hook-test.ps1 answers that).
  Click-Middle $switches[0]

  # A second modifier on the trigger, and the other hand. Each one is checked in
  # the file straight after the click that caused it: this is the section that
  # says a row applies itself, one row at a time.
  Click-Middle ((Get-Button-ByText $session.wnd "Shift")[0])
  Check (Wait-FileLine $session "trigger_mods=5") "the trigger gained Shift on the click itself: mods 1 became 5"
  Click-Middle ((Get-Button-ByText $session.wnd "right button")[0])
  Check (Wait-FileLine $session "trigger_button=right") "and the other button, on its own click"
  Save-Shot $session.wnd (Join-Path $shotDir "settings-trigger.png")

  # One step up on the size, one step down on the distance. The steps are ten
  # logical pixels and two pixels: what the ruler and the gesture are measured in.
  $plus = @(Get-Button-ByText $session.wnd $script:Plus | Sort-Object { (Get-Rect $_).Top })
  $minus = @(Get-Button-ByText $session.wnd $script:Minus | Sort-Object { (Get-Rect $_).Top })
  Click-Middle $plus[0]
  Check (Wait-FileLine $session "wheel_radius=160\.0") "the size stepped up by ten, unasked: 150 became 160"
  Click-Middle $minus[1]
  Check (Wait-FileLine $session "drag_threshold=6") "and the distance stepped down by two: 8 became 6"
  Save-Shot $session.wnd (Join-Path $shotDir "settings-steppers.png")

  $saved = Saved-Text $session
  Check ($saved -match "wheel=1") "and the switch, clicked twice, is where it started"
} finally {
  Close-Settings $session
}

Write-Host "== the same file, read back by a second window =="
# The window is a draft: it shows what the file says, and nothing else. A number
# that only lives until the window closes is a number nobody can set.
Stop-Shaiwin
$session = Open-Settings "reread" ($base + "wheel_radius=180.0`ndrag_threshold=14`ntrigger_mods=3`n")
try {
  Save-Shot $session.wnd (Join-Path $shotDir "settings-reread.png")
  Assert-NoCut $session.state "with the longest numbers it can be handed"
  $saved = Saved-Text $session
  Check ($saved -match "wheel_radius=180\.0") "the size it was handed is still the size: 180.0"
  Check ($saved -match "drag_threshold=14") "and so is the distance: 14"
  Check ($saved -match "trigger_mods=3") "and the trigger: Alt+Ctrl"
} finally {
  Close-Settings $session
}

Write-Host "== how tall the page is on this screen =="
# The claim a screenshot cannot make on its own: the last thing on the tallest
# page is on the screen, not below it. That page is the dial's own - five cards,
# and the ends of the distance stepper are the lowest controls on it - and the
# frame is sized to it, which is why clicking a shorter section leaves air rather
# than moving the bottom edge.
# `the_window_is_as_tall_as_its_tallest_section` in crates/app/src/settings_ui.rs
# walks the same layout at 100, 125, 150 and 200 percent; this is the same claim
# against the pixels of the monitor this session happens to have.
Stop-Shaiwin
$session = Open-Settings "height" $base
try {
  $r = Get-Rect $session.wnd
  $v = Get-VirtualBounds
  $low = 0
  $lowest = [IntPtr]::Zero
  foreach ($class in "PanWen.Settings.Button", "PanWen.Settings.Key", "PanWen.Settings.Switch") {
    foreach ($h in @(Get-Children $session.wnd $class)) {
      $b = (Get-Rect $h).Bottom
      if ($b -gt $low) { $low = $b; $lowest = $h }
    }
  }
  if (-not (Test-Window $lowest)) { Fail "the tallest page has no controls on it" }
  $lr = Get-Rect $lowest
  $tall = $r.Bottom - $r.Top
  Write-Host "  window $($r.Left),$($r.Top)-$($r.Right),$($r.Bottom) at scale $($session.scale), desktop ends at $($v.Bottom)"
  Check ($lr.Bottom -le $v.Bottom) "the last control of the tallest page is inside the screen (its bottom $($lr.Bottom) of $($v.Bottom))"
  Check ($lr.Top -ge $r.Top -and $lr.Bottom -le $r.Bottom) "and it is inside the window that made it"
  Check ($r.Bottom -le $v.Bottom) "the whole window is on the screen ($tall tall on a $($v.Bottom - $v.Top) desktop)"
} finally {
  Close-Settings $session
}

Write-Host "== the window list is a section of this page, and a row answers a click =="
# The second half of the shortcut story: the seats that Ctrl+2 made are managed on
# this page, in the section the left column names Windows. There is no list window
# of its own any more, so this walks the whole path from the tray to a row without
# a hand - and without a second door to open first.
function Fav-Count([string]$text) {
  if ($text -match "(?m)^favorite_slots=(.*)$") {
    return @(@($Matches[1] -split "," | Where-Object { $_ -ne "" })).Count
  }
  return 0
}
Stop-Shaiwin
$root = Join-Path $env:TEMP "shaizero-settings-windows"
$state = New-Sandbox $root $base
Initialize-Fixtures $root
$proc = Start-Shaiwin $Exe $state
Start-Sleep -Milliseconds 2200
$fixture = New-Fixture "Listed" 0
try {
  Invoke-Wm (Get-AppWindow) $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
  if (-not (Wait-Until { Test-Window (Get-Settings) } 5000)) { Fail "the settings window never opened" }
  Start-Sleep -Milliseconds 900
  $session = @{ state = $state; proc = $proc; wnd = Get-Settings; scale = [W]::GetDpiForWindow((Get-Settings)) / 96.0 }
  Goto-Section $session.wnd $script:NavId.Windows
  Save-Shot $session.wnd (Join-Path $shotDir "settings-windows.png")

  # Four cells per card, and four cards however many the desktop is running: the
  # frame is the same height with two windows on the dial as with two hundred,
  # which is the promise the scroll is there to keep.
  $fav = @(Get-Button-ByText $session.wnd "Favourite")
  $pin = @(Get-Button-ByText $session.wnd "Pin")
  $off = @(Get-Button-ByText $session.wnd "Off")
  Check ($fav.Count -ge 1) "the list has a row to press ($($fav.Count) shown)"
  Check ($fav.Count -le 4) "and never more rows than the frame holds ($($fav.Count))"
  Check ($pin.Count -eq $fav.Count -and $off.Count -eq $fav.Count) "every row carries all three verbs"

  # A verb is the whole setting. It writes the file as it is pressed, and the list
  # is then read back from the file rather than from what the click was asked to
  # do - which is why a strike can land on every window of an application at once.
  Check ((Fav-Count (Saved-Text $session)) -eq 0) "nothing is on the dial before a row is pressed"
  Click-Middle $fav[0]
  Check (Wait-Until { (Fav-Count (Saved-Text $session)) -eq 1 } 4000) "pressing 'Favourite' wrote the seat"
  Click-Middle $fav[0]
  Check (Wait-Until { (Fav-Count (Saved-Text $session)) -eq 0 } 4000) "and pressing it again took the seat back out"

  # The name cell is the one cell that decides nothing: it opens the editor, and
  # Escape alone has to be enough to leave the row exactly as it was.
  $blank = @(Get-Button-ByText $session.wnd "Add a tag")
  if ($blank.Count -ge 1) {
    Click-Middle $blank[0]
    if (Wait-Until { Test-Window (Get-TagBox) } 4000) {
      Check $true "a click on a name cell opened the box that labels it"
      Save-Shot (Get-TagBox) (Join-Path $shotDir "window-tag-box.png")
      Press-Escape (Get-TagBox)
      Check (Wait-Until ({ -not (Test-Window (Get-TagBox)) }) 4000) "and Escape put it away again"
    } else {
      Check $false "a click on the first row's name cell opened nothing"
    }
  } else {
    Write-Host "  SKIPPED: every row on the dial already carries a tag"
  }
} finally {
  Stop-Shaiwin
  Remove-Fixtures
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "== the language row, a folder pack, and picking another language =="
Stop-Shaiwin
$langRoot = Join-Path $env:TEMP "shaizero-settings-lang"
$state = New-Sandbox $langRoot ($base + "lang=auto`n")
$langDir = Join-Path $state "ShaiZero\languages"
New-Item -ItemType Directory -Force $langDir | Out-Null

# Spelled from code points, not literals: these files carry no byte-order mark,
# so Windows PowerShell reads them in the ANSI code page and a non-ASCII string
# here would arrive at the app already mangled.
$frenchName = "Fran$([char]0xE7)ais"
$zhName = "$([char]0x7B80)$([char]0x4F53)$([char]0x4E2D)$([char]0x6587)"
$enFolder = $script:EnFolder
# A word this program ships a name for, translated by hand: the door at the foot
# of the page. `settings_save` used to be the line here, and there is no Save left
# to name.
$frFolder = "Fichiers de langue"

# A language the program does not ship, found in the folder the panel opens. The
# two lines it carries are enough to prove the file was read and named.
$pack = "_name = $frenchName`nlang_folder = $frFolder`n"
[System.IO.File]::WriteAllText((Join-Path $langDir "fr.txt"), $pack, (New-Object System.Text.UTF8Encoding $true))

$proc = Start-Shaiwin $Exe $state
Start-Sleep -Milliseconds 2200
$app = Get-AppWindow
if (-not (Test-Window $app)) { Fail "the app window is not there" }
Invoke-Wm $app $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
if (-not (Wait-Until { Test-Window (Get-Settings) } 5000)) {
  Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
  Fail "the settings window never opened"
}
Start-Sleep -Milliseconds 900
$session = @{ state = $state; proc = $proc; wnd = Get-Settings; scale = [W]::GetDpiForWindow((Get-Settings)) / 96.0 }
try {
  # The names of the languages are on that page, and this session starts in
  # whatever the machine speaks - so the column is clicked by its number, not by
  # a word this script cannot know in advance.
  Goto-Section $session.wnd $script:NavId.Language
  Save-Shot $session.wnd (Join-Path $shotDir "settings-language.png")

  # Starting up fills the folder with a complete copy of every shipped language,
  # which is what a new translation is written into rather than typed out.
  # Read as UTF-8 on purpose: this shell's default code page is 936, and a GBK
  # reading of a Chinese pack eats the `=` off the end of half its lines.
  #
  # How long a full pack is is not something this script can know: the table is
  # one line longer or shorter every time a word is added or a button deleted,
  # and the floor written here used to read 60. What the two copies can prove is
  # that they were laid out from the same table - key for key - and that the
  # wheel's own words survived the trip while the deleted Save button did not.
  $packKeys = @{}
  foreach ($shipped in "en.txt", "zh-CN.txt") {
    $file = Join-Path $langDir $shipped
    if (-not (Test-Path $file)) { Fail "startup left no $shipped in the folder" }
    $lines = [System.IO.File]::ReadAllLines($file, [System.Text.Encoding]::UTF8)
    $entries = @($lines | Where-Object { $_.Trim() -ne "" -and -not $_.StartsWith("#") })
    $assign = @($entries | Where-Object { $_ -match "=" })
    if ($entries.Count -ne $assign.Count) { Fail "$shipped has a line that is neither a comment nor a key" }
    if ($assign.Count -lt 40) { Fail "$shipped is a truncated copy, not a pack ($($assign.Count) lines)" }
    $packKeys[$shipped] = @($assign | ForEach-Object { ($_ -split '=', 2)[0].Trim() } | Sort-Object)
  }
  $drift = @(Compare-Object $packKeys['en.txt'] $packKeys['zh-CN.txt'])
  if ($drift.Count -gt 0) { Fail "the two shipped packs are not the same list of keys: $($drift[0].InputObject)" }
  Check ($packKeys['en.txt'] -contains 'settings_wheel_note_brief') "the wheel's own note is in the copy to translate"
  Check ($packKeys['en.txt'] -contains 'settings_nav_trigger') "and so are the names of the five pages"
  Check ($packKeys['en.txt'] -contains 'win_fav') "and the words a window row is pressed with"
  foreach ($gone in 'settings_save', 'settings_group_wheel', 'settings_wheel_note', 'panel_fav') {
    Check (-not ($packKeys['en.txt'] -contains $gone)) "the word that was deleted did not come back: $gone"
  }
  Write-Host "  the shipped languages were exported to the folder"

  # The row offers the system, both shipped languages and the file in the folder,
  # and every one of them is named the way it names itself.
  foreach ($name in "English", $zhName, $frenchName) {
    $row = Get-Button-ByText $session.wnd $name
    if ($row.Count -ne 1) { Fail "the language row has $name of them: $($row.Count)" }
  }
  Write-Host "  the row offers the system, both shipped languages and the folder's"

  Write-Host "== picking a language: the words and the file move together =="
  # The row that answers a click by changing every word on screen is the clearest
  # case of a live panel, and the only one a screenshot can prove: no Save, no
  # reopen, the door at the foot of the page just says something else.
  Click-Middle ((Get-Button-ByText $session.wnd $frenchName)[0])
  Check (Wait-FileLine $session "lang=fr") "the pick was written on the click: lang=fr"
  $log = @(Read-SharedText (Join-Path $session.state "ShaiZero\shaizero.log")) -join "`n"
  Check ($log -match "language is now fr") "and the switch was logged"
  $door = @(Get-Button-ByText $session.wnd $frFolder).Count
  Check ($door -eq 1) "and the panel is speaking the pack now: the door reads $frFolder ($door of them)"
  Save-Shot $session.wnd (Join-Path $shotDir "settings-french.png")

  Write-Host "== picking English, still with nothing to press =="
  Click-Middle ((Get-Button-ByText $session.wnd "English")[0])
  Check (Wait-FileLine $session "lang=en") "the pick was written: lang=en"
  $log = @(Read-SharedText (Join-Path $session.state "ShaiZero\shaizero.log")) -join "`n"
  Check ($log -match "language is now en") "and the switch was logged"

  # A reopened panel is drawn in the language it just chose, which is the whole
  # claim of switching without a restart: the dial and this window read one table.
  Invoke-Wm (Get-AppWindow) $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
  if (-not (Wait-Until { Test-Window (Get-Settings) } 5000)) {
    Fail "the settings window never reopened"
  }
  Start-Sleep -Milliseconds 900
  $session.wnd = Get-Settings
  Goto-Section $session.wnd $script:NavId.Language
  Save-Shot $session.wnd (Join-Path $shotDir "settings-english.png")
  Assert-NoCut $session.state "in English"
  if ((Get-Button-ByText $session.wnd $enFolder).Count -ne 1) { Fail "the door to the language files has no English name" }
  Write-Host "  the panel came back in English"
} finally {
  Close-Settings $session
  Remove-Item $langRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host "done: $shotDir"
if ($script:fails -eq 0) { Write-Host "PASS" }
else { Write-Host "FAIL: $($script:fails) check(s)"; exit 1 }
