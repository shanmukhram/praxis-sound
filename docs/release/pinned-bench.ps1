# Runs `praxis bench` pinned to one logical CPU (WP-Z render-speed measurement; no audio).
# usage: pinned-bench.ps1 -Exe praxis.exe -Cpu 2 -Out result.json [-BenchArgs "--runs 3"]
param([string]$Exe, [int]$Cpu, [string]$Out, [string]$BenchArgs = "--runs 3")
$env:PRAXIS_KEYCHAIN = 'off'
$psi = New-Object Diagnostics.ProcessStartInfo
$psi.FileName = $Exe
$psi.Arguments = "--json bench $BenchArgs"
$psi.RedirectStandardOutput = $true
$psi.UseShellExecute = $false
$p = [Diagnostics.Process]::Start($psi)
$p.ProcessorAffinity = [IntPtr]([int64]1 -shl $Cpu)
$p.PriorityClass = 'AboveNormal'
$text = $p.StandardOutput.ReadToEnd()
$p.WaitForExit()
[IO.File]::WriteAllText($Out, $text)
