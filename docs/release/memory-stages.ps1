# Memory of the installed app at three stages (WP-Z2): a sound open and not yet played ("idle"),
# after its first Play (then stopped), and after closing it (New sound). No audio is heard:
# PRAXIS_AUDIO_OUTPUT=fake. Buttons are pressed through UI Automation.
# usage: memory-stages.ps1 -Exe <installed praxis-app.exe> -PraxisHome <home with one sound> -Settle 20
param([string]$Exe, [string]$PraxisHome, [int]$Settle = 20)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$env:PRAXIS_HOME = $PraxisHome
$env:PRAXIS_AUDIO_OUTPUT = 'fake'
$env:PRAXIS_KEYCHAIN = 'off'
Remove-Item Env:ANTHROPIC_API_KEY -ErrorAction SilentlyContinue

function Get-Ids([int]$rootId) {
  $all = Get-CimInstance Win32_Process
  @($rootId) + @($all | Where-Object { $_.Name -eq 'msedgewebview2.exe' -and $_.CommandLine -match 'webview-exe-name=praxis-app\.exe' } | ForEach-Object { $_.ProcessId })
}

function Get-StageMemory([int]$rootId, [string]$stage) {
  $ids = Get-Ids $rootId
  $app = 0; $tot = 0; $priv = 0
  foreach ($id in $ids) {
    $pp = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$id"
    if ($null -eq $pp) { continue }
    $tot += $pp.WorkingSetPrivate
    $priv += (Get-Process -Id $id).PrivateMemorySize64
    if ($id -eq $rootId) { $app = $pp.WorkingSetPrivate }
  }
  [ordered]@{
    stage = $stage
    app_private_working_set_mb = [math]::Round($app / 1MB, 1)
    total_private_working_set_mb = [math]::Round($tot / 1MB, 1)
    total_private_bytes_mb = [math]::Round($priv / 1MB, 1)
    processes = $ids.Count
  }
}

function Invoke-Button([IntPtr]$hwnd, [string]$name) {
  $root = [Windows.Automation.AutomationElement]::FromHandle($hwnd)
  $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, $name)
  for ($k = 0; $k -lt 100; $k++) {
    $el = $root.FindFirst([Windows.Automation.TreeScope]::Descendants, $cond)
    if ($el) {
      $el.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
      return
    }
    Start-Sleep -Milliseconds 200
  }
  throw "no button named '$name'"
}

$p = Start-Process -FilePath $Exe -PassThru
while ($p.MainWindowHandle -eq 0) { Start-Sleep -Milliseconds 100; $p.Refresh() }
$out = @()
Start-Sleep -Seconds $Settle
$out += Get-StageMemory $p.Id 'sound open, not yet played (idle)'
Invoke-Button $p.MainWindowHandle 'Play take 1'
Start-Sleep -Seconds 3
Invoke-Button $p.MainWindowHandle 'Stop take 1'
Start-Sleep -Seconds $Settle
$out += Get-StageMemory $p.Id 'after its first Play (stopped)'
Invoke-Button $p.MainWindowHandle 'New sound'
Start-Sleep -Seconds $Settle
$out += Get-StageMemory $p.Id 'after closing it (New sound)'
Get-Process -Id (Get-Ids $p.Id) -ErrorAction SilentlyContinue | Sort-Object Id -Descending | Stop-Process -Force
$out | ConvertTo-Json -Depth 3
