# Drives the REAL app window with a REAL Caddy and a REAL MariaDB (task 7c): installs both from the Runtimes page into a
# throw-away LUNA_HOME (about 120 MB of downloads), starts them, opens the live log viewer on each and reads what it shows
# (lunaRun's own note, Caddy's own output, the tabs of MariaDB's output and error log), and then closes the window with both
# running to see the quit progress overlay ("Stopping your services") and that the app exits with both stopped. Windows only.
# (Adopting or stopping a leftover cannot be run on Windows: the Job Objects end every service with the app, so none is ever
# left over; those paths are tested with real processes in the Rust tests.)
#
#   pnpm tauri build --no-bundle
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/ui-check/real-logs-and-quit.ps1 -Exe <repo>/target/release/lunarun.exe -HomeParent <empty folder> -OutDir <folder>
#
# Needs internet, ports 2119 and 3306 or the next free one for the database, and no caddy.exe or mariadbd.exe running. The data
# folder under -HomeParent is called "lüna home" (a space and a non-ASCII letter on purpose); the script builds that name itself.
param(
  [Parameter(Mandatory)] [string]$Exe,
  [Parameter(Mandatory)] [string]$HomeParent,
  [Parameter(Mandatory)] [string]$OutDir,
  [int]$MinWidth = 1125,
  [int]$MinHeight = 750
)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System; using System.Runtime.InteropServices; using System.Drawing; using System.Drawing.Imaging;
public static class Win {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
  public struct RECT { public int Left, Top, Right, Bottom; }
  public static void Capture(IntPtr h, string path) {
    RECT r; GetWindowRect(h, out r);
    using (var bmp = new Bitmap(r.Right - r.Left, r.Bottom - r.Top)) {
      using (var g = Graphics.FromImage(bmp)) { IntPtr hdc = g.GetHdc(); PrintWindow(h, hdc, 2); g.ReleaseHdc(hdc); }
      bmp.Save(path, ImageFormat.Png);
    }
  }
}
"@
[void][Win]::SetProcessDPIAware()
New-Item -ItemType Directory -Force $OutDir | Out-Null
$LunaHome = Join-Path (Resolve-Path -LiteralPath $HomeParent).Path ("l" + [char]0x00FC + "na home")
New-Item -ItemType Directory -Force $LunaHome | Out-Null
"LUNA_HOME = $LunaHome"
$env:LUNA_HOME = $LunaHome

$failures = New-Object System.Collections.Generic.List[string]
function Check($ok, $what) { if ($ok) { "  ok   $what" } else { "  FAIL $what"; $script:failures.Add($what) } }

$AE = [Windows.Automation.AutomationElement]
$CT = [Windows.Automation.ControlType]
$script:h = [IntPtr]::Zero
function Root { $AE::FromHandle($script:h) }
function Find($name, $type) {
  $c = New-Object Windows.Automation.AndCondition(
    (New-Object Windows.Automation.PropertyCondition($AE::NameProperty, $name)),
    (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $type)))
  (Root).FindFirst([Windows.Automation.TreeScope]::Descendants, $c)
}
function FindLike($pattern, $type) {
  $c = New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $type)
  foreach ($e in (Root).FindAll([Windows.Automation.TreeScope]::Descendants, $c)) { if ($e.Current.Name -match $pattern) { return $e } }
  $null
}
function Press($el) {
  # A button with aria-pressed (the Logs toggle) offers the Toggle pattern, not Invoke; a tab offers SelectionItem.
  foreach ($pattern in @([Windows.Automation.InvokePattern]::Pattern, [Windows.Automation.TogglePattern]::Pattern, [Windows.Automation.SelectionItemPattern]::Pattern)) {
    $found = $null
    if ($el.TryGetCurrentPattern($pattern, [ref]$found)) {
      if ($pattern -eq [Windows.Automation.InvokePattern]::Pattern) { $found.Invoke() }
      elseif ($pattern -eq [Windows.Automation.TogglePattern]::Pattern) { $found.Toggle() }
      else { $found.Select() }
      return
    }
  }
  throw "nothing to press on '$($el.Current.Name)'"
}
function Texts {
  try {
    $all = (Root).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
    $names = foreach ($e in $all) { $n = $e.Current.Name; if ($n) { $n } }
    $names | Select-Object -Unique
  } catch { @() }
}
function TextLike($pattern) { @(Texts | Where-Object { $_ -match $pattern }) }
function WaitFor([scriptblock]$test, [int]$seconds, [string]$what) {
  for ($i = 0; $i -lt $seconds * 4; $i++) { if (& $test) { return $true }; Start-Sleep -Milliseconds 250 }
  "  (timed out after ${seconds}s waiting for: $what)"
  $false
}
function Shot($name) { [Win]::Capture($script:h, (Join-Path $OutDir "$name.png")) }
function Procs { @(Get-Process -Name caddy, mariadbd -ErrorAction SilentlyContinue | ForEach-Object { $_.ProcessName + ":" + $_.Id }) }
function InstallFromRuntimes($tabPattern, $buttonPattern, $uninstallPattern, $seconds) {
  Press (Find "Runtimes" $CT::Hyperlink)
  Start-Sleep -Seconds 3
  $tab = FindLike $tabPattern $CT::TabItem
  if (-not $tab) { return $false }
  Press $tab
  Start-Sleep -Seconds 1
  $null = WaitFor { $script:btn = FindLike $buttonPattern $CT::Button; [bool]$script:btn } 30 "an Install button"
  if (-not $script:btn) { return $false }
  "  pressing: " + $script:btn.Current.Name
  Press $script:btn
  WaitFor { [bool](FindLike $uninstallPattern $CT::Button) } $seconds "the install to finish"
}

if (Procs) { "caddy or mariadbd is already running on this computer: stop it first."; exit 5 }
$started = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
for ($i = 0; $i -lt 90 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh() }
$script:h = $p.MainWindowHandle
if ($script:h -eq [IntPtr]::Zero) { "NO WINDOW"; if (-not $p.HasExited) { $p.Kill() }; exit 2 }
"window up after {0:N1}s (pid {1})" -f ((Get-Date) - $started).TotalSeconds, $p.Id
[void][Win]::MoveWindow($script:h, 40, 40, 1400, 900, $true)
Start-Sleep -Seconds 4

"--- 1. install Caddy and MariaDB from the Runtimes page"
Check (InstallFromRuntimes "^Caddy" "^Install Caddy" "^Uninstall Caddy" 300) "Caddy installed"
Check (InstallFromRuntimes "^MariaDB" "^Install MariaDB" "^Uninstall MariaDB" 900) "MariaDB installed"
if ($failures.Count -gt 0) { $p.Kill(); "INSTALL FAILED: $($failures -join '; ')"; exit 6 }

"--- 2. start both from the Services page"
Press (Find "Services" $CT::Hyperlink)
Start-Sleep -Seconds 3
$null = WaitFor { [bool](Find "Start Caddy" $CT::Button) } 20 "Start Caddy"
Press (Find "Start Caddy" $CT::Button)
Check (WaitFor { (Texts) -contains "Running" } 60 "Caddy running") "Caddy runs"
$null = WaitFor { [bool](FindLike "^Start MariaDB" $CT::Button) } 20 "Start MariaDB"
$dbStart = FindLike "^Start MariaDB" $CT::Button
$dbName = $dbStart.Current.Name -replace "^Start ", ""
"  database row: $dbName"
Press $dbStart
$sawSetup = WaitFor { [bool](TextLike "^Setting up its data folder") } 30 "the first-start progress line"
Check $sawSetup "the first start of the database says it is setting up its data folder"
Shot "real-1-database-first-start"
Check (WaitFor { @(Texts | Where-Object { $_ -eq "Running" }).Count -ge 1 -and [bool](FindLike "^Stop $dbName" $CT::Button) } 180 "the database running") "$dbName runs"
Check ((Procs | Where-Object { $_ -match "^mariadbd" }).Count -eq 1) "one mariadbd runs"
Check ((Procs | Where-Object { $_ -match "^caddy" }).Count -eq 1) "one caddy runs"

"--- 3. the log viewer on Caddy"
Press (Find "Show the logs of Caddy" $CT::Button)
Check (WaitFor { [bool](TextLike "^Logs of Caddy$") } 20 "the Caddy log panel") "the log panel opens"
$null = WaitFor { [bool](TextLike "\[lunaRun\] starting") } 20 "lunaRun's note"
Check ([bool](TextLike "\[lunaRun\] starting")) "it shows lunaRun's own note: [lunaRun] starting"
Check (WaitFor { [bool](TextLike "admin endpoint|serving initial configuration|autosaved|started") } 20 "Caddy's own output") ("it shows what Caddy printed itself: " + ((TextLike "admin endpoint|serving initial configuration") | Select-Object -First 1))
Check ([bool](Find "Output" $CT::TabItem)) "there is an Output tab"
Shot "real-2-caddy-logs-light-1400"

"--- 4. the log viewer on the database (output and its error log are two tabs)"
Press (Find "Show the logs of $dbName" $CT::Button)
$null = WaitFor { [bool](TextLike "^Logs of $dbName") } 20 "the database log panel"
$errTab = $null
$null = WaitFor { $script:errTab = FindLike "error log" $CT::TabItem; [bool]$script:errTab } 30 "the error log tab"
Check ([bool]$script:errTab) ("the database has its error log as a tab: " + $(if ($script:errTab) { $script:errTab.Current.Name } else { "(none)" }))
if ($script:errTab) {
  Press $script:errTab
  Start-Sleep -Seconds 1
  Check (WaitFor { [bool](TextLike "ready for connections|InnoDB|Server socket|mariadbd") } 20 "a line of the error log") ("the error log tab shows the server's own words: " + ((TextLike "ready for connections|InnoDB") | Select-Object -First 1))
}
Shot "real-3-database-error-log-light-1400"

"--- 5. quit with both running: the progress overlay, then the exit"
Press (Find "Hide the logs of $dbName" $CT::Button)
$exeName = [IO.Path]::GetFileNameWithoutExtension($Exe)
$closing = Get-Date
[void]$p.CloseMainWindow()
$sawOverlay = $false
$shot = $false
for ($i = 0; $i -lt 400 -and -not $p.HasExited; $i++) {
  $seen = TextLike "^Stopping your services"
  if ($seen) {
    $sawOverlay = $true
    if (-not $shot) {
      try { Shot "real-4-quit-overlay"; $shot = $true } catch { }
      $groupsSeen = TextLike "Web server|PHP|Databases|of \d+ stopped|Waiting for"
      "  overlay shows: " + ($groupsSeen -join " | ")
    }
  }
  Start-Sleep -Milliseconds 50
}
$exited = $p.WaitForExit(60000)
"  the app exited {0:N1}s after the window was closed (exit code {1})" -f ((Get-Date) - $closing).TotalSeconds, $p.ExitCode
Check $sawOverlay "the window stayed open and showed 'Stopping your services' while the services stopped"
Check $exited "the app exited by itself"
Start-Sleep -Milliseconds 800
Check (-not (Procs)) "no caddy or mariadbd is left"
$errLog = Get-ChildItem -LiteralPath (Join-Path $LunaHome "logs") -Filter "mariadb-*.err" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($errLog) {
  $tail = Get-Content -LiteralPath $errLog.FullName -Tail 15
  Check ([bool]($tail -match "Shutdown complete|shutdown complete")) "the database shut down cleanly (its error log says so)"
}
Check (@(Get-ChildItem -LiteralPath (Join-Path $LunaHome "run") -Filter "*.pid" -ErrorAction SilentlyContinue).Count -eq 0) "no PID file is left"

""
if ($failures.Count -eq 0) { "ALL CHECKS PASSED"; exit 0 } else { "FAILED: " + $failures.Count; $failures | ForEach-Object { "  - $_" }; exit 1 }
