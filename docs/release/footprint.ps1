# Cold start and idle memory of the installed app (WP-Z footprint measurement). No audio: PRAXIS_AUDIO_OUTPUT=fake.
# usage: footprint.ps1 -Exe <praxis-app.exe> -PraxisHome <empty PRAXIS_HOME> -Runs 6 -IdleSeconds 20
param([string]$Exe, [string]$PraxisHome, [int]$Runs = 6, [int]$IdleSeconds = 20)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$env:PRAXIS_HOME = $PraxisHome
$env:PRAXIS_AUDIO_OUTPUT = 'fake'
$env:PRAXIS_KEYCHAIN = 'off'
Remove-Item Env:ANTHROPIC_API_KEY -ErrorAction SilentlyContinue

function Get-Tree([int]$rootId) {
  $all = Get-CimInstance Win32_Process
  $ids = @($rootId); $added = $true
  while ($added) {
    $added = $false
    foreach ($p in $all) { if ($ids -contains $p.ParentProcessId -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId; $added = $true } }
  }
  # WebView2 runs in msedgewebview2.exe processes started for this exe (not always as children).
  foreach ($p in $all) { if ($p.Name -eq 'msedgewebview2.exe' -and $p.CommandLine -match 'webview-exe-name=praxis-app\.exe' -and -not ($ids -contains $p.ProcessId)) { $ids += $p.ProcessId } }
  $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue }
}

$results = @()
for ($i = 1; $i -le $Runs; $i++) {
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $p = Start-Process -FilePath $Exe -PassThru
  $window = $null; $ui = $null
  while ($sw.Elapsed.TotalSeconds -lt 20) {
    $p.Refresh()
    if (-not $window -and $p.MainWindowHandle -ne 0) { $window = $sw.Elapsed.TotalMilliseconds }
    if ($window) {
      try {
        $root = [Windows.Automation.AutomationElement]::FromHandle($p.MainWindowHandle)
        $cond = New-Object Windows.Automation.PropertyCondition([Windows.Automation.AutomationElement]::NameProperty, 'New sound')
        if ($root.FindFirst([Windows.Automation.TreeScope]::Descendants, $cond)) { $ui = $sw.Elapsed.TotalMilliseconds; break }
      } catch {}
    }
    Start-Sleep -Milliseconds 15
  }
  $mem = $null
  if ($i -eq $Runs) {
    Start-Sleep -Seconds $IdleSeconds
    $tree = Get-Tree $p.Id
    $own = Get-Process -Id $p.Id
    $mem = [ordered]@{
      app_working_set_mb = [math]::Round($own.WorkingSet64 / 1MB, 1)
      app_private_mb = [math]::Round($own.PrivateMemorySize64 / 1MB, 1)
      processes = $tree.Count
      tree_working_set_mb = [math]::Round((($tree | Measure-Object WorkingSet64 -Sum).Sum) / 1MB, 1)
      tree_private_mb = [math]::Round((($tree | Measure-Object PrivateMemorySize64 -Sum).Sum) / 1MB, 1)
      tree_private_working_set_mb = [math]::Round((($tree | ForEach-Object { (Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Filter "IDProcess=$($_.Id)").WorkingSetPrivate } | Measure-Object -Sum).Sum) / 1MB, 1)
      names = ($tree | Group-Object ProcessName | ForEach-Object { "$($_.Name) x$($_.Count)" }) -join ', '
    }
  }
  $tree = Get-Tree $p.Id
  $tree | Sort-Object Id -Descending | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
  Start-Sleep -Seconds 2
  $results += [ordered]@{ run = $i; window_ms = [math]::Round($window); ui_ready_ms = [math]::Round($ui); memory = $mem }
}
$results | ConvertTo-Json -Depth 4
