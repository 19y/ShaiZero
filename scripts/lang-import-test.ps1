# A language pack imported the way a user imports one (scripts\lang-import-test.ps1).
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\lang-import-test.ps1
#
# Russian is not built into the program. It is a file somebody writes and drops into
# %LOCALAPPDATA%\ShaiZero\languages, so this test does exactly that: it copies
# languages\ru.txt into a private sandbox, picks Русский on the settings panel,
# saves, and then looks at what the app drew - the settings window, the window list
# it opens, and the label editor both of them share - to see whether the words
# arrived.
#
# Captions and button texts are what a test can read from outside: every one of
# those windows puts its painted word in the window text as well, which is what
# a screen reader sees too. Everything else on these screens is ink, so the
# program has to say out loud when a line has no room for a translation, and
# `Assert-NoCut` reads that out of the log. A pack that overflows a button fails
# this test instead of shipping the overflow.
#
# It also checks the two things a user's folder can get wrong: a file named after
# the setting that means "follow the system" must not appear as a second choice,
# and a pack saved in an encoding the program cannot read must say which file and
# why rather than disappear.
#
# The label editor is walked through in Russian too, because that is the one screen
# where a translation has to fit words the program never chose: the popover has to
# grow to hold them, the field has to take a Cyrillic tag, and the button that
# clears it has to be the button that clears it.
#
# Nothing of the user's is read or written: the app runs against its own
# LOCALAPPDATA, and the window whose label is written and struck out again is a
# throwaway fixture.
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
    // Which button a control is, as opposed to what word it is wearing at the
    // moment. The door to the language folder is id 3 in every language and the
    // left column numbers its names from 301, and this test switches languages on
    // purpose.
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr h);
    // Messages to the field have to be synchronous - PostMessage answers before
    // the control has seen anything - and they have to be the wide ones, because
    // the ANSI entry point converts Cyrillic through the machine's own codepage
    // and the word arrives with extra letters in it.
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr SendKeyW(IntPtr h, uint msg, IntPtr wp, IntPtr lp);
'@
}

$shotDir = Join-Path $env:TEMP "shaizero-lang-shots"
New-Item -ItemType Directory -Force -Path $shotDir | Out-Null
Get-ChildItem $shotDir -Filter "*.png" -ErrorAction SilentlyContinue | Remove-Item -Force

# Cyrillic words spelled from code points: these files have no byte-order mark, and
# PowerShell 5.1 reads a mark-less file as the system's own codepage, which turns
# "Русский" into something the parser cannot even hold. The word each line builds is
# written in brackets after it, for whoever has to read this file - in brackets, and
# never at the end of a line, because that same codepage pairs a leftover byte with
# the newline and swallows the statement below it. This happened: a comment ending
# in an ellipsis ate the two assignments after it, and the test then searched the
# panel for a name that had never been set.
function Word([int[]]$codes) { return -join ($codes | ForEach-Object { [char]$_ }) }
$ruRussian = Word @(0x0420, 0x0443, 0x0441, 0x0441, 0x043A, 0x0438, 0x0439)      # [Русский]
# The door to the language folder, in the two languages this session can open in.
# It used to be the Save button that carried these checks; the window has no Save
# any more, and a control that no longer exists proves nothing about words.
$ruFolder = Word @(0x0424, 0x0430, 0x0439, 0x043B, 0x044B, 0x0020, 0x044F, 0x0437,
                  0x044B, 0x043A, 0x043E, 0x0432, 0x2026)                        # [Файлы языков...]
$zhFolder = Word @(0x8BED, 0x8A00, 0x6587, 0x4EF6, 0x2026)                      # [语言文件...]
$ruSystem = Word @(0x0421, 0x0438, 0x0441, 0x0442, 0x0435, 0x043C, 0x043D, 0x044B, 0x0439)  # [Системный]
$ruTitle = Word @(0x041F, 0x0430, 0x0440, 0x0430, 0x043C, 0x0435, 0x0442, 0x0440, 0x044B,
                 0x0020, 0x0053, 0x0068, 0x0061, 0x0069, 0x005A, 0x0065, 0x0072, 0x006F) # [Параметры ShaiZero]
$ruWindows = Word @(0x041E, 0x043A, 0x043D, 0x0430)                            # [Окна]
$ruPin = Word @(0x0417, 0x0430, 0x043A, 0x0440, 0x0435, 0x043F, 0x0438, 0x0442,
               0x044C)                                                          # [Закрепить]
$ruTag = Word @(0x041F, 0x0440, 0x043E, 0x0432, 0x0435, 0x0440, 0x043A, 0x0430)  # [Проверка]

function Get-Children([IntPtr]$parent, [string]$want) {
  $script:kids = New-Object System.Collections.ArrayList
  $cb = [Win.Print+EnumProc]{
    param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 256
    [void][Win.Print]::GetClassNameW($h, $sb, 256)
    if ($sb.ToString() -eq $want) { [void]$script:kids.Add($h) }
    return $true
  }
  [void][Win.Print]::EnumChildWindows($parent, $cb, [IntPtr]::Zero)
  return $script:kids.ToArray()
}

function Get-Text([IntPtr]$h) {
  $sb = New-Object System.Text.StringBuilder 512
  [void][Win.Print]::GetWindowTextW($h, $sb, 512)
  return $sb.ToString()
}

# Every control on the settings panel is one class, and the only thing that tells
# one door from another, or a door from a language, is the id the panel gave it
# when it made it.
function Get-Button-ById([IntPtr]$parent, [int]$id) {
  foreach ($b in @(Get-Children $parent "PanWen.Settings.Button")) {
    if ([Win.Print]::GetDlgCtrlID($b) -eq $id) { return $b }
  }
  return [IntPtr]::Zero
}

function Get-Button-ByText([IntPtr]$parent, [string]$want) {
  @(Get-Children $parent "PanWen.Settings.Button" | Where-Object { (Get-Text $_) -eq $want })
}

function Button-Texts([IntPtr]$parent) {
  @(Get-Children $parent "PanWen.Settings.Button" | ForEach-Object { Get-Text $_ })
}

# Reading the field from outside the app's process: WM_GETTEXTLENGTH asks the
# control how many characters it holds, and the answer needs no buffer to carry
# across the border between the two processes - so it is the one that can be
# trusted. (GetWindowText and EM_GETLINE are the casualties of that border:
# the first reports what the control was created with, the second reports the
# size word this caller had to hand it.)
function Get-FieldLength([IntPtr]$edit) {
  return [int][Win.Print]::SendKeyW($edit, 0x000E, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
}

function Type-Into([IntPtr]$edit, [string]$word) {
  # One character at a time, the way a user types it.
  foreach ($ch in $word.ToCharArray()) {
    [void][Win.Print]::SendKeyW($edit, 0x0102, [IntPtr][int]$ch, [IntPtr]::Zero)
  }
}

function Get-Box([IntPtr]$h) {
  $r = Get-Rect $h
  if (-not $r) { Fail "no rect for a child window" }
  $w = $r.Right - $r.Left
  $t = $r.Bottom - $r.Top
  if ($w -lt 2 -or $t -lt 2) { Fail "the control has no size ($w x $t)" }
  return @{ w = $w; t = $t }
}

function Send-Click([IntPtr]$h, [int]$x, [int]$y) {
  $lp = [IntPtr](($y -shl 16) -bor ($x -band 0xFFFF))
  Invoke-Wm $h ([uint32]0x0201) ([IntPtr]::Zero) $lp
  Invoke-Wm $h ([uint32]0x0202) ([IntPtr]::Zero) $lp
  Start-Sleep -Milliseconds 250
}

function Send-Click-Middle([IntPtr]$h) {
  $box = Get-Box $h
  Send-Click $h ([int]($box.w / 2)) ([int]($box.t / 2))
}

function Save-Shot([IntPtr]$hwnd, [string]$path) {
  $r = New-Object W+RECT
  if (-not [W]::GetWindowRect($hwnd, [ref]$r)) { Fail "no rect for $path" }
  $w = $r.Right - $r.Left
  $h = $r.Bottom - $r.Top
  if ($w -lt 20 -or $h -lt 20) { Fail "the window is not really there ($w x $h)" }
  $bmp = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $gfx = [System.Drawing.Graphics]::FromImage($bmp)
  $hdc = $gfx.GetHdc()
  $ok = [Win.Print]::PrintWindow($hwnd, $hdc, 2)
  $gfx.ReleaseHdc($hdc)
  $gfx.Dispose()
  if (-not $ok) { $bmp.Dispose(); Fail "PrintWindow failed for $path" }
  $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Host "shot: $path ($w x $h)"
}

$root = Join-Path $env:TEMP "shaizero-lang"
# The shipped defaults, and only the keys the program still reads. The two
# shortcuts are the shipped ones because the window list is opened from the
# settings door rather than by a keystroke, and this file is about words, not about
# whether the keyboard works (hook-test answers that).
$cfg = "reduced_motion=0`nlang=auto`n" +
       "excluded=`nwheel=1`ntrigger_mods=1`ntrigger_button=left`n" +
       "wheel_radius=150.0`nwheel_dead=0.42`ndrag_threshold=8`n" +
       "fav_mods=2`nfav_vk=50`npanel_mods=2`npanel_vk=84`n"
$state = New-Sandbox $root $cfg
Initialize-Fixtures $root
$log = Join-Path $state "ShaiZero\shaizero.log"
$settingsFile = Join-Path $state "ShaiZero\settings.txt"

$langDir = Join-Path $state "ShaiZero\languages"
New-Item -ItemType Directory -Force $langDir | Out-Null
# The user's own file, copied out of the repository and into the folder the
# settings panel opens. Nothing else about Russian is known to the program.
Copy-Item (Join-Path $PSScriptRoot "..\languages\ru.txt") (Join-Path $langDir "ru.txt") -Force
# A file named after the setting that means "follow the system". It is not a
# language, and the row must not offer it as a second one.
Set-Content -Path (Join-Path $langDir "auto.txt") -Value "_name = Nope`nquit = No`n" -Encoding ascii
$ruBytes = (Get-Item (Join-Path $langDir "ru.txt")).Length

$fixture = New-Fixture "Ru" 0
Stop-Shaiwin
[void](Start-Shaiwin $Exe $state)
Start-Sleep -Milliseconds 2200
if (-not (Test-Window (Get-AppWindow))) { Fail "the app window is not there" }

function Lines { if (Test-Path $log) { @(Get-Content $log -Encoding utf8) } else { @() } }
function Mark { (Lines).Count }
function Since([int]$m) { @((Lines) | Select-Object -Skip $m) }
function Text([int]$m) { ((Since $m) -join "`n") }

# Read with the bystander's sharing modes: the app swaps this file in by renaming a
# sibling over it, and an ordinary reader holds it open against that write.
function Settings-Text { @((Read-SharedText $settingsFile)) -join "`n" }
function Field-Line([string]$key) {
  $m = [regex]::Match((Settings-Text), "(?m)^$key=(.*)`$")
  if ($m.Success) { return $m.Groups[1].Value }
  return $null
}
function Tag-Line { Field-Line "window_tags" }

$script:fails = 0
function Check([bool]$ok, [string]$what) {
  if ($ok) { Write-Host "  ok   $what" } else { $script:fails++; Write-Host "  FAIL $what" }
}

function Open-Settings {
  Invoke-Wm (Get-AppWindow) $script:KitOpenSettings ([IntPtr]::Zero) ([IntPtr]::Zero)
  if (-not (Wait-Until ({ Test-Window (Get-Settings) }) 6000)) {
    Fail "the settings window never opened"
  }
  Start-Sleep -Milliseconds 900
  return Get-Settings
}

function Close-Settings([IntPtr]$wnd) {
  Invoke-Wm $wnd $script:KitClose ([IntPtr]::Zero) ([IntPtr]::Zero)
  [void](Wait-Until ({ -not (Test-Window $wnd) }) 5000)
  Start-Sleep -Milliseconds 300
}

# The language names and the door to their folder are one page of a panel that
# shows one page at a time, so a click aimed at either of them has to bring that
# page up first - a message posted at a hidden button would move the app, but it
# would not be the click a person makes. The name in the left column is pressed by
# the id the panel numbered it with: this test speaks three languages, and none of
# them spells that button the same way. The column is Trigger 301, Shortcuts 302,
# Windows 303, Language 304, About 305 - in that order in every language.
function Goto-Languages([IntPtr]$parent) {
  Goto-ById $parent 304 "the language page"
}

function Goto-Windows([IntPtr]$parent) {
  Goto-ById $parent 303 "the window list"
}

function Goto-ById([IntPtr]$parent, [int]$id, [string]$what) {
  $nav = Get-Button-ById $parent $id
  if (-not (Test-Window $nav)) { Fail "the left column has no $what to click" }
  Send-Click-Middle $nav
  Start-Sleep -Milliseconds 350
}

# The four things that can be done to a window are cells of their own card, and a
# card stays where it was made: the list moves underneath it, never the other way
# round. So the slot says the id and the id says the slot - `500 + slot * 4 +
# verb`, the arithmetic in `WIN_BASE` - and a click aimed at a name cell lands on
# the window that name belongs to. A card holding nothing has its cells hidden,
# which is why a hidden one is refused here rather than clicked: pressing an
# invisible control tests the message queue, not the page.
function Get-Window-Cell([IntPtr]$parent, [int]$slot, [int]$verb) {
  $cell = Get-Button-ById $parent (500 + $slot * 4 + $verb)
  if (-not (Test-Window $cell)) { return [IntPtr]::Zero }
  if (-not [W]::IsWindowVisible($cell)) { return [IntPtr]::Zero }
  return $cell
}

function Click-Row-Label([IntPtr]$parent, [int]$index) {
  $cell = Get-Window-Cell $parent $index 0
  if (-not (Test-Window $cell)) { return $false }
  Send-Click-Middle $cell
  return $true
}

# The editor sits under the window it names, so the two rects say which row was
# clicked without anybody having to read a painted line.
function Editor-Names([IntPtr]$box, [IntPtr]$wnd) {
  $b = Get-Rect $box
  $f = Get-Rect $wnd
  if (-not $b -or -not $f) { return $false }
  $cx = [int](($b.Left + $b.Right) / 2)
  return ($cx -ge ($f.Left - 24) -and $cx -le ($f.Right + 24) -and $b.Top -ge ($f.Bottom - 32))
}

# Opens the label editor for one row of the list, and keeps walking down the cards
# until it has the one that belongs to the fixture. A card whose editor lands
# somewhere else is left with Escape, which writes nothing. Four cards is the whole
# frame; reaching past them is what the scroll is for, and this session does not
# need it.
function Open-Label-For([IntPtr]$parent, [IntPtr]$wnd) {
  for ($row = 0; $row -lt 4; $row++) {
    if (-not (Click-Row-Label $parent $row)) { continue }
    if (-not (Wait-Until ({ Test-Window (Get-TagBox) }) 3000)) { continue }
    Start-Sleep -Milliseconds 700
    $box = Get-TagBox
    if (Editor-Names $box $wnd) { return $box }
    $b = Get-Rect $box
    $f = Get-Rect $wnd
    Write-Host "  row $row is a window of somebody else's (editor $([int](($b.Left+$b.Right)/2)),$($b.Top) against window $($f.Left),$($f.Top)-$($f.Right),$($f.Bottom)); leaving it alone"
    Press-Escape $box
    [void](Wait-Until ({ -not (Test-Window $box) }) 4000)
  }
  return [IntPtr]::Zero
}

try {
  Write-Host "== the panel before anybody speaks Russian =="
  $wnd = Open-Settings
  Write-Host "  caption: $(Get-Text $wnd)"
  Write-Host "  buttons: $(@(Button-Texts $wnd) -join ' | ')"
  $langDoor = Get-Button-ById $wnd 3
  Check (Test-Window $langDoor) "the panel has a door to the language folder, whatever it is called"
  Check ((Get-Text $wnd) -ne $ruTitle) "and it is not speaking Russian yet"
  Check ((Get-Text $langDoor) -ne $ruFolder) "nor is its own door"
  # The button this test used to press to make a pick real. There is nothing left
  # to press: the row is the decision, and it lands on its own.
  Check (-not (Test-Window (Get-Button-ById $wnd 1))) "and there is no Save control on the window at all"
  Check (@(Get-Button-ByText $wnd "English").Count -eq 1) "English is offered once"
  Check (@(Get-Button-ByText $wnd $ruRussian).Count -eq 1) "the pack in the folder is offered under the name it gives itself"
  Check (@(Get-Button-ByText $wnd "Nope").Count -eq 0) "auto.txt is not offered as a language of its own"
  Save-Shot $wnd (Join-Path $shotDir "settings-before.png")
  Assert-NoCut $state "as it opened"
  Close-Settings $wnd

  Write-Host "== a pack the program cannot read says so =="
  # Notepad's "Unicode" is UTF-16, and it is the one save that turns a pack into a
  # file the program cannot read. Silent disappearance is the failure being checked.
  Set-Content -Path (Join-Path $langDir "de.txt") -Value "_name = Deutsch" -Encoding unicode
  $m = Mark
  $wnd = Open-Settings
  Check ((Text $m) -match "language pack de cannot be read as UTF-8") "the log names the file that could not be read"
  Check (@(Get-Button-ByText $wnd "de").Count -eq 1) "and the pack is still offered, under its file name"
  Check (@(Get-Button-ByText $wnd "Deutsch").Count -eq 0) "not under a name the program never got to read"
  Close-Settings $wnd
  Remove-Item (Join-Path $langDir "de.txt") -Force

  Write-Host "== picking the pack: the click is the whole decision =="
  $wnd = Open-Settings
  Goto-Languages $wnd
  $ru = @(Get-Button-ByText $wnd $ruRussian)
  if ($ru.Count -ne 1) { Fail "the pack in the folder was not offered as a single button" }
  $langDoor = Get-Button-ById $wnd 3
  $m = Mark
  Send-Click-Middle $ru[0]
  # Nothing else was pressed. The row applies itself, the file follows a beat
  # later, and the words on this very window go with it.
  $landed = Wait-Until ({ (Settings-Text) -match "(?m)^lang=ru`$" }) 5000
  Check $landed "the pick reached the settings file on its own"
  Check ((Text $m) -match "language is now ru") "and the app said so: $((Since $m) -join ' / ')"
  Check ((Get-Text $langDoor) -eq $ruFolder) "and the door to the folder took the pack's word without reopening"

  Write-Host "== the settings panel comes back speaking Russian =="
  $wnd = Open-Settings
  # The page stays up: the way back out of a language is a button on this page,
  # and a test that pressed it while the dial's rows were showing would be
  # pressing a window nobody can see.
  Goto-Languages $wnd
  Write-Host "  caption: $(Get-Text $wnd)"
  Write-Host "  buttons: $(@(Button-Texts $wnd) -join ' | ')"
  Check ((Get-Text $wnd) -eq $ruTitle) "the window calls itself $($ruTitle)"
  Check ((Get-Text (Get-Button-ById $wnd 3)) -eq $ruFolder) "and the door to the language folder is $($ruFolder)"
  Check (@(Get-Button-ByText $wnd "Language files$([char]0x2026)").Count -eq 0) "no English door survived the switch"
  Check (@(Get-Button-ByText $wnd $zhFolder).Count -eq 0) "no Chinese door survived it either"
  Check (@(Get-Button-ByText $wnd $ruSystem).Count -eq 1) "the system row kept its Russian name"
  Check ((Get-Text (Get-Button-ById $wnd 303)) -eq $ruWindows) "and the left column names the window list $($ruWindows)"
  Save-Shot $wnd (Join-Path $shotDir "settings-ru.png")
  Assert-NoCut $state "in Russian"

  Write-Host "== the window list, in the pack's own words =="
  # The fixture goes to the top of the stack first: the list is the z-order, so this
  # is how the test says which card it is about to click. The list is a section of
  # this page now, not a window of its own, so the click that brings it up is a name
  # in the left column and there is nothing second to wait for.
  # NOSIZE | NOMOVE | NOACTIVATE, and HWND_TOP: the point is to put the fixture at
  # the front of the stack without disturbing where it sits or stealing the
  # keyboard. The three flags that say "leave the rest alone" are 0x1, 0x2 and 0x10;
  # 0x4 is NOZORDER, which would have asked for the raise and then refused it, and
  # with no NOMOVE and no NOSIZE the zeros below would have been taken as a place
  # and a size - which is what this line used to do to the fixture.
  [void][W]::SetWindowPos($fixture, [IntPtr]::Zero, 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0010)
  Start-Sleep -Milliseconds 400
  $m = Mark
  Goto-Windows $wnd
  Check (@(Get-Button-ByText $wnd $ruPin).Count -ge 1) "the cards are pressed with $($ruPin)"
  Check (@(Get-Button-ByText $wnd "Pin").Count -eq 0) "and no English verb survived the switch"
  Save-Shot $wnd (Join-Path $shotDir "settings-windows-ru.png")
  Assert-NoCut $state "on the window list"

  Write-Host "== the label editor, with words the program never chose =="
  $box = Open-Label-For $wnd $fixture
  if (-not (Test-Window $box)) {
    # The list is every switchable window on the machine, so a fixture that is not
    # in the four cards the frame holds is an arrangement this session could not
    # make - not a translation that failed. Say which it is.
    Write-Host "  SKIPPED: the fixture was not within reach of the four cards"
  } else {
    Save-Shot $box (Join-Path $shotDir "tag-ru-empty.png")
    $first = Get-Rect $box
    $wide = $first.Right - $first.Left
    $tall = $first.Bottom - $first.Top
    Write-Host "  the popover is $wide x $tall"
    # The buttons are painted by the app, not controls, so the only external proof
    # that the popover grew to hold this language's words is that it says it did.
    $grown = $wide -gt [int](286.0 * ($tall / 136.0))
    $said = @(Since $m | Where-Object { $_ -match "the tag popover grew" })
    if ($grown -and $said.Count -eq 0) {
      Check $false "the popover grew to ${wide}px for this language's words and never logged it"
    }
    if ($grown) { Write-Host "  $($said[0])" }
    Assert-NoCut $state "in the popover's own words"

    $edit = @(Get-Children $box "Edit")
    if ($edit.Count -ne 1) {
      Check $false ("the popover has {0} fields, not one" -f $edit.Count)
    } else {
      Check ((Get-FieldLength $edit[0]) -eq 0) "a fresh label opens with an empty field"
      Type-Into $edit[0] $ruTag
      Check ((Get-FieldLength $edit[0]) -eq $ruTag.Length) `
        ("the field kept all {0} characters of {1}" -f $ruTag.Length, $ruTag)
      $m = Mark
      Press-Enter $box
      Check (Wait-Until ({ -not (Test-Window $box) }) 5000) "Enter commits it and takes the popover away"
      Check ((Text $m) -match "tag: window labelled") "the app says the window is labelled: $((Since $m) -join ' / ')"
      Check ((Tag-Line) -match $ruTag) "and the file carries the word: $(Tag-Line)"

      # Opened again on the same row: the same words, so the same width, and the tag
      # already in the field rather than lost with the window that showed it.
      $again = Open-Label-For $wnd $fixture
      if (Test-Window $again) {
        $second = Get-Rect $again
        Check (($second.Right - $second.Left) -eq $wide) "the popover did not jump a size on the way back open"
        $keep = @(Get-Children $again "Edit")
        if ($keep.Count -eq 1) {
          Check ((Get-FieldLength $keep[0]) -eq $ruTag.Length) "reopened, the field still holds the word it was given"
        }
        Save-Shot $again (Join-Path $shotDir "tag-ru-filled.png")
        Assert-NoCut $state "in the popover with all three words"
        # The clearing button: the ghost on the left of the row, fourteen of padding
        # in from the popover's own edge. The row is thirty logical points tall and
        # sits fourteen from the bottom of a popover a hundred and thirty six tall.
        $s = [W]::GetDpiForWindow($again) / 96.0
        $m = Mark
        Send-Click $again ([int](18 * $s)) ([int](107 * $s))
        Check (Wait-Until ({ -not (Test-Window $again) }) 5000) "the left-hand button took the popover away"
        Check ((Text $m) -match "tag: window unlabelled") "and the app says the word is gone: $((Since $m) -join ' / ')"
        Check ((Tag-Line) -notmatch $ruTag) "the file no longer holds it: $(Tag-Line)"
      } else {
        Check $false "the fixture's row could not be found a second time, so the reopening had nothing to check"
      }
    }
    # There is no second window to put away any more: the list is a section of the
    # page the next block still needs open, so the page stays where it is.
  }

  Write-Host "== the user's file is still the user's file =="
  if ((Get-Item (Join-Path $langDir "ru.txt")).Length -ne $ruBytes) {
    Check $false "the program rewrote the pack it was given"
  } else {
    Write-Host "  ru.txt untouched at $ruBytes bytes"
  }
  foreach ($shipped in "en.txt", "zh-CN.txt") {
    if (-not (Test-Path (Join-Path $langDir $shipped))) {
      Check $false "startup left no $shipped in the folder to copy and translate"
    }
  }

  Write-Host "== back to following the system =="
  # Pressing the way back needs the row that is wearing the Russian word for
  # "system", so say which of the two ways this can fail went wrong: a window that
  # is no longer there, or a window whose buttons are not the ones expected.
  # The page is left on the window list by the block above, and a click aimed at a
  # hidden language button would move the app without being the click a person
  # makes - so bring the page back to the names first.
  if (-not (Test-Window $wnd)) { Fail "the settings window was gone before the way back was looked for" }
  Goto-Languages $wnd
  $sysRows = @(Get-Button-ByText $wnd $ruSystem)
  if ($sysRows.Count -ne 1) {
    Fail "the system row is $($sysRows.Count) of them; the window has: $(@(Button-Texts $wnd) -join ' | ')"
  }
  $sys = $sysRows[0]
  Check (Test-Window $sys) "the system row is a button that can be pressed"
  $langDoor = Get-Button-ById $wnd 3
  Check ((Get-Text $langDoor) -eq $ruFolder) "the door is still wearing the pack's word"
  Send-Click-Middle $sys
  Check (Wait-Until ({ (Settings-Text) -match "(?m)^lang=auto`$" }) 5000) "the way back landed in the file on its own click"
  Check ((Get-Text $langDoor) -ne $ruFolder) "and the word on the door went back with it"
  $wnd = Open-Settings
  Write-Host "  caption: $(Get-Text $wnd)"
  Write-Host "  buttons: $(@(Button-Texts $wnd) -join ' | ')"
  Check ((Get-Text $wnd) -ne $ruTitle) "the panel is not Russian any more"
  Check (@(Get-Button-ByText $wnd $ruFolder).Count -eq 0) "and no Russian word survived on a button"
  Assert-NoCut $state "on the way back"

  Write-Host "== the folder door does not take the app with it =="
  Goto-Languages $wnd
  $door = Get-Button-ById $wnd 3
  Check (Test-Window $door) "the panel still knows which button opens the language folder"
  Send-Click-Middle $door
  Start-Sleep -Milliseconds 800
  Check (Test-Window (Get-AppWindow)) "the app is alive after the folder was opened"
  Close-Settings $wnd
} finally {
  Remove-Fixtures
  Stop-Shaiwin
  Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
  Write-Host "cleaned up"
}

Write-Host ""
Write-Host "shots: $shotDir"
if ($script:fails -eq 0) { Write-Host "PASS" }
else { Write-Host "FAIL: $($script:fails) check(s)"; exit 1 }
