# aIwalk helper, runs as SYSTEM (scheduled task "aIwalkAgent"). ASCII only: PowerShell 5.1 reads CP950.
# It only runs the fixed jobs installed next to it; a request file names a job and carries no code.
$dir = '\\host.lan\Data\.aiwalk'
$jobs = @{ 'cleanup' = 'C:\ProgramData\aiwalk\cleanup.ps1' }
while ($true) {
    try {
        Set-Content "$dir\alive" (Get-Date -Format o)
        if (Test-Path "$dir\request") {
            $name = ((Get-Content "$dir\request" -TotalCount 1) + '').Trim()
            Remove-Item "$dir\request" -Force
            if ($jobs.ContainsKey($name)) {
                Set-Content "$dir\status" $name
                $out = & powershell -NoProfile -ExecutionPolicy Bypass -File $jobs[$name] *>&1 | Out-String
            } else {
                $out = "unknown job: $name"
            }
            Set-Content "$dir\result.tmp" $out
            Move-Item "$dir\result.tmp" "$dir\result.txt" -Force
            Remove-Item "$dir\status" -Force
        }
    } catch {}
    Start-Sleep 5
}
