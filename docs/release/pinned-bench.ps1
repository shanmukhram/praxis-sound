# Runs `praxis bench` pinned to a set of logical CPUs (WP-Z render-speed measurement; no audio).
# usage: pinned-bench.ps1 -Exe praxis.exe -Cpus "2" | "0-11" | "12-19" -Out result.json [-BenchArgs "--runs 3"]
# (-Cpu N is kept for one CPU.) On the i5-14600K, CPUs 0-11 are the 6 performance cores
# (2 threads each) and 12-19 the 8 efficiency cores.
param([string]$Exe, [int]$Cpu = -1, [string]$Cpus = "", [string]$Out, [string]$BenchArgs = "--runs 3")
$env:PRAXIS_KEYCHAIN = 'off'
if ($Cpu -ge 0) { $Cpus = "$Cpu" }
$mask = [int64]0
foreach ($part in $Cpus.Split(',')) {
  $r = $part.Split('-')
  $lo = [int]$r[0]; $hi = if ($r.Count -gt 1) { [int]$r[1] } else { $lo }
  for ($c = $lo; $c -le $hi; $c++) { $mask = $mask -bor ([int64]1 -shl $c) }
}
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = "--json bench $BenchArgs"
$psi.RedirectStandardOutput = $true
$psi.UseShellExecute = $false
$p = [Diagnostics.Process]::Start($psi)
$p.ProcessorAffinity = [IntPtr]$mask
$p.PriorityClass = 'AboveNormal'
$text = $p.StandardOutput.ReadToEnd()
$p.WaitForExit()
[IO.File]::WriteAllText($Out, $text)
