# Drives the REAL app window (not a browser): opens the Services page, reads what the window actually shows through UI
# Automation, takes screenshots at 1400 wide and at the app's minimum size in the dark theme, then closes the window the
# way a user does and times how long the process takes to exit. Unit tests mock the backend, so this is what proves a new
# command is permitted (capabilities) and that the exit hook does not trap the app. Windows only.
#
#   pnpm tauri build --no-bundle
#   node scripts/ui-check/seed-home.mjs <parent>
#   powershell -File scripts/ui-check/real-window.ps1 -Exe target\release\lunarun.exe -HomeParent <parent> -OutDir <folder>
#
# The app's minimum window is 900x600 LOGICAL pixels: with 125% display scaling that is 1125x750 physical pixels, which is
# what this script uses (a smaller window than the app allows shows clipping that a user can never see). Change
# $MinWidth and $MinHeight for another scaling (physical = logical x scale; the scale is AppliedDPI / 96).
param(
  [Parameter(Mandatory)] [string]$Exe,
  [Parameter(Mandatory)] [string]$HomeParent,
  [Parameter(Mandatory)] [string]$OutDir,
  [switch]$QuitOnly,
  [int]$MinWidth = 1125,
  [int]$MinHeight = 750
)
# Drives the real app window: opens the Services page, reads what the window actually shows through UI Automation,
# takes screenshots, then closes the window and times how long the process takes to exit.
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
# The seeded folder has a space and a non-ASCII letter in its name: find it here instead of passing it as an argument.
$LunaHome = (Get-ChildItem -LiteralPath $HomeParent -Directory | Select-Object -First 1).FullName
"LUNA_HOME = $LunaHome"
$env:LUNA_HOME = $LunaHome
$started = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
for ($i = 0; $i -lt 90 -and $p.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 500; $p.Refresh() }
$h = $p.MainWindowHandle
if ($h -eq 0) { "NO WINDOW after $((Get-Date) - $started)"; if (-not $p.HasExited) { $p.Kill() }; exit 2 }
"window up after {0:N1}s (pid {1})" -f ((Get-Date) - $started).TotalSeconds, $p.Id
[void][Win]::MoveWindow($h, 40, 40, 1400, 900, $true)
Start-Sleep -Seconds 4
$AE = [Windows.Automation.AutomationElement]
$CT = [Windows.Automation.ControlType]
function Root { $AE::FromHandle($h) }
function Find($name, $type) {
  $c = New-Object Windows.Automation.AndCondition(
    (New-Object Windows.Automation.PropertyCondition($AE::NameProperty, $name)),
    (New-Object Windows.Automation.PropertyCondition($AE::ControlTypeProperty, $type)))
  (Root).FindFirst([Windows.Automation.TreeScope]::Descendants, $c)
}
function Invoke($el) { $el.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke() }
function Texts {
  $all = (Root).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  $names = foreach ($e in $all) { $n = $e.Current.Name; if ($n) { $n } }
  $names | Select-Object -Unique
}

if (-not $QuitOnly) {
  $link = $null
  for ($i = 0; $i -lt 20 -and -not $link; $i++) { $link = Find "Services" $CT::Hyperlink; if (-not $link) { Start-Sleep -Milliseconds 500 } }
  if (-not $link) { "COULD NOT FIND THE Services LINK"; (Texts) | Select-Object -First 40; $p.Kill(); exit 3 }
  Invoke $link
  Start-Sleep -Seconds 4
  "--- what the window shows on Services:"
  (Texts) | Where-Object { $_ -match "Caddy|PHP|MySQL|MariaDB|Version|workers|Not assigned|Stopped|Running|9100|3307|2119|first start|password|next update|No services|Could not|earlier session" } | ForEach-Object { "   $_" }
  [Win]::Capture($h, (Join-Path $OutDir "real-services-light-1400.png"))
  # The theme switch: find its buttons by what the page calls them.
  $dark = $null
  $everything = (Root).FindAll([Windows.Automation.TreeScope]::Descendants, [Windows.Automation.Condition]::TrueCondition)
  foreach ($e in $everything) { if ($e.Current.Name -match "^Dark" -and -not $dark) { $dark = $e; "--- theme switch: '" + $e.Current.Name + "' (" + $e.Current.ControlType.ProgrammaticName + ")" } }
  if ($dark) {
    try { $dark.GetCurrentPattern([Windows.Automation.SelectionItemPattern]::Pattern).Select() }
    catch { try { Invoke $dark } catch { "could not activate the theme switch: $_" } }
    Start-Sleep -Seconds 1
  } else { "NO THEME SWITCH FOUND" }
  [void][Win]::MoveWindow($h, 40, 40, $MinWidth, $MinHeight, $true)
  Start-Sleep -Seconds 2
  [Win]::Capture($h, (Join-Path $OutDir "real-services-dark-min.png"))
}

# Close the window as a user would, and time the exit.
$closing = Get-Date
[void]$p.CloseMainWindow()
$exited = $p.WaitForExit(30000)
if ($exited) { "closed: the process exited {0:N1}s after the window was closed (exit code {1})" -f ((Get-Date) - $closing).TotalSeconds, $p.ExitCode }
else { "STILL RUNNING 30s after the window was closed"; $p.Kill(); exit 4 }
