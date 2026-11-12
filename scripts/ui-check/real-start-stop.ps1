# Drives the REAL app window with a REAL service: installs Caddy from the Runtimes page into a throw-away LUNA_HOME (a real
# download of about 18 MB), then on the Services page proves Start, the Running state arriving by event, Restart, Stop, a
# taken admin port (the problem panel, who holds it), and finally a QUIT WITH A SERVICE RUNNING: closing the window must stop
# Caddy and exit. Unit tests mock the backend, so this is what proves the four new commands are permitted (capabilities) and
# that the whole chain works with a real program. Windows only.
#
#   pnpm tauri build --no-bundle
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/ui-check/real-start-stop.ps1 -Exe target\release\lunarun.exe -HomeParent <empty folder> -OutDir <folder>
#
# The data folder made under -HomeParent is called "lüna home" (a space and a non-ASCII letter on purpose, NFR-07). The
# script finds it itself instead of taking it as an argument: a `ü` in a command line gets mangled between shells.
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
  try { $el.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
  catch { $el.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() }
}
function Texts {
  $all = (Root).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $names = foreach ($e in $all) { $n = $e.Current.Name; if ($n) { $n } }
  $names | Select-Object -Unique
}
function WaitFor([scriptblock]$test, [int]$seconds, [string]$what) {
  for ($i = 0; $i -lt $seconds * 4; $i++) { if (& $test) { return $true }; Start-Sleep -Milliseconds 250 }
  "  (timed out after ${seconds}s waiting for: $what)"
  $false
}
function Shot($name) { [Win]::Capture($script:h, (Join-Path $OutDir "$name.png")) }
function PortOpen($port) {
  $c = New-Object Net.Sockets.TcpClient
  try { $c.Connect("127.0.0.1", $port); $true } catch { $false } finally { $c.Close() }
}
function CaddyPids { @(Get-Process -Name caddy -ErrorAction SilentlyContinue | ForEach-Object { $_.Id }) }
function SetTheme($prefix) {
  $e = FindLike "^$prefix" $CT::RadioButton
  if (-not $e) { $e = FindLike "^$prefix" $CT::Button }
  if ($e) { Press $e; Start-Sleep -Milliseconds 700 } else { "  (no $prefix theme switch found)" }
}

if (CaddyPids) { "A caddy process is already running on this computer: stop it first so the check is not confused."; exit 5 }
$started = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
for ($i = 0; $i -lt 90 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh() }
$script:h = $p.MainWindowHandle
if ($script:h -eq [IntPtr]::Zero) { "NO WINDOW"; if (-not $p.HasExited) { $p.Kill() }; exit 2 }
"window up after {0:N1}s (pid {1})" -f ((Get-Date) - $started).TotalSeconds, $p.Id
[void][Win]::MoveWindow($script:h, 40, 40, 1400, 900, $true)
Start-Sleep -Seconds 4

# ---------------------------------------------------------------- 1. Install a real Caddy from the Runtimes page
"--- 1. install Caddy from the Runtimes page"
$link = $null
for ($i = 0; $i -lt 20 -and -not $link; $i++) { $link = Find "Runtimes" $CT::Hyperlink; if (-not $link) { Start-Sleep -Milliseconds 500 } }
if (-not $link) { "COULD NOT FIND THE Runtimes LINK"; (Texts) | Select-Object -First 40; $p.Kill(); exit 3 }
Press $link
Start-Sleep -Seconds 3
$tab = FindLike "^Caddy" $CT::TabItem
if (-not $tab) { "COULD NOT FIND THE Caddy TAB"; (Texts) | Select-Object -First 60; $p.Kill(); exit 3 }
Press $tab
Start-Sleep -Seconds 1
$install = $null
WaitFor { $script:install = FindLike "^Install Caddy" $CT::Button; [bool]$script:install } 30 "an Install Caddy button" | Out-Null
if (-not $script:install) { "NO INSTALL BUTTON FOR CADDY"; (Texts) | Select-Object -First 60; $p.Kill(); exit 3 }
"  pressing: " + $script:install.Current.Name
Press $script:install
$installed = WaitFor { [bool](FindLike "^Uninstall Caddy" $CT::Button) } 300 "Caddy to be installed"
Check $installed "Caddy was installed by the app (an Uninstall button appeared)"
$caddyExe = Get-ChildItem -LiteralPath (Join-Path $LunaHome "runtimes\caddy") -Recurse -Filter caddy.exe -ErrorAction SilentlyContinue | Select-Object -First 1
Check ([bool]$caddyExe) "caddy.exe is on disk inside the data folder"
if (-not $installed) { $p.Kill(); "INSTALL FAILED: $($failures -join '; ')"; exit 6 }

# ---------------------------------------------------------------- 2. The Services page, Start
"--- 2. Services page: Start"
$link = Find "Services" $CT::Hyperlink
Press $link
Start-Sleep -Seconds 3
$null = WaitFor { $script:startButton = Find "Start Caddy" $CT::Button; [bool]$script:startButton } 20 "the Start Caddy button"
Check ([bool]$script:startButton) "a Start button for Caddy is shown"
Shot "real-1-stopped-light-1400"
Press $script:startButton
$running = WaitFor { (Texts) -contains "Running" } 60 "the Running state"
Check $running "the row says Running (the state arrived by event)"
Check ([bool](Find "Stop Caddy" $CT::Button)) "Stop and Restart are offered while it runs"
Check ([bool](Find "Restart Caddy" $CT::Button)) "Restart is offered while it runs"
Check (-not (Find "Start Caddy" $CT::Button)) "Start is not offered while it runs"
$pids1 = CaddyPids
Check ($pids1.Count -eq 1) "exactly one caddy.exe is running (pid $($pids1 -join ','))"
Check (PortOpen 2119) "Caddy's admin port 2119 answers"
try { $r = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:2119/config/" -TimeoutSec 5; Check ($r.StatusCode -eq 200) "the admin API answers 200" } catch { Check $false "the admin API answers ($_)" }
Check ((Texts) -contains "Caddy started") "a toast confirmed: Caddy started"
Shot "real-2-running-light-1400"

# ---------------------------------------------------------------- 3. Restart
"--- 3. Restart"
Press (Find "Restart Caddy" $CT::Button)
$newPid = WaitFor { $now = CaddyPids; ($now.Count -eq 1) -and ($now[0] -ne $pids1[0]) -and ((Texts) -contains "Running") } 60 "a new caddy process"
Check $newPid "Restart replaced the process (old $($pids1 -join ','), new $((CaddyPids) -join ','))"
Check (PortOpen 2119) "the admin port answers again"

# ---------------------------------------------------------------- 4. Stop
"--- 4. Stop"
Press (Find "Stop Caddy" $CT::Button)
$stopped = WaitFor { (-not (CaddyPids)) -and ((Texts) -contains "Stopped") } 60 "Caddy to stop"
Check $stopped "Stop ended the process and the row says Stopped"
Check (-not (PortOpen 2119)) "the admin port is free again"
Check ((Texts) -contains "Caddy stopped") "a toast confirmed: Caddy stopped"

# ---------------------------------------------------------------- 5. A taken port
"--- 5. a taken admin port"
$holder = New-Object Net.Sockets.TcpListener([Net.IPAddress]::Loopback, 2119)
$holder.Start()
$null = WaitFor { $script:startButton = Find "Start Caddy" $CT::Button; [bool]$script:startButton } 20 "the Start Caddy button"
Press $script:startButton
$problem = WaitFor { (Texts) -contains "Caddy did not start" } 30 "the problem panel"
Check $problem "a problem panel says Caddy did not start"
Shot "real-3-port-problem-light-1400"
$said = (Texts) | Where-Object { $_ -match "^Port 2119 is" }
Check ([bool]$said) ("it says who holds the port: " + ($said -join " | "))
Check (-not [bool](FindLike "^Use another port" $CT::Button)) "Caddy is not offered another port (it has only one)"
Check (-not (CaddyPids)) "no caddy.exe was left behind by the failed start"
SetTheme "Dark"
[void][Win]::MoveWindow($script:h, 40, 40, $MinWidth, $MinHeight, $true)
Start-Sleep -Seconds 2
Shot "real-3-port-problem-dark-min"
[void][Win]::MoveWindow($script:h, 40, 40, 1400, 900, $true)
SetTheme "Light"
$holder.Stop()
Start-Sleep -Milliseconds 800

# ---------------------------------------------------------------- 6. Quit with a service running
"--- 6. quit with Caddy running"
Press (Find "Start Caddy" $CT::Button)
$running = WaitFor { (Texts) -contains "Running" } 60 "Running again"
Check $running "Caddy runs again once the port is free"
$pids2 = CaddyPids
Check ($pids2.Count -eq 1) "one caddy.exe runs before the quit (pid $($pids2 -join ','))"
$closing = Get-Date
[void]$p.CloseMainWindow()
$exited = $p.WaitForExit(60000)
if ($exited) { "  the process exited {0:N1}s after the window was closed (exit code {1})" -f ((Get-Date) - $closing).TotalSeconds, $p.ExitCode }
else { "  STILL RUNNING 60s after the window was closed"; $p.Kill() }
Check $exited "closing the window with a service running exits the app"
Start-Sleep -Milliseconds 500
Check (-not (CaddyPids)) "caddy.exe is gone after the quit"
Check (-not (PortOpen 2119)) "the admin port is free after the quit"
Check (-not (Test-Path (Join-Path $LunaHome "run\caddy.pid"))) "its PID file is gone"

""
if ($failures.Count -eq 0) { "ALL CHECKS PASSED"; exit 0 } else { "FAILED: " + $failures.Count; $failures | ForEach-Object { "  - $_" }; exit 1 }
