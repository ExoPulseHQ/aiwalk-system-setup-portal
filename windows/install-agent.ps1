# One-time setup of the aIwalk helper, run as Administrator. ASCII only.
$ErrorActionPreference = 'Stop'
$dest = 'C:\ProgramData\aiwalk'
New-Item -ItemType Directory $dest -Force | Out-Null
# only SYSTEM and Administrators may change the jobs
icacls $dest /inheritance:r /grant:r 'SYSTEM:(OI)(CI)F' 'Administrators:(OI)(CI)F' 'Users:(OI)(CI)RX' | Out-Null
Copy-Item '\\host.lan\Data\.aiwalk\agent.ps1', '\\host.lan\Data\.aiwalk\cleanup.ps1' $dest -Force
$a = New-ScheduledTaskAction -Execute powershell.exe -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File $dest\agent.ps1"
$t = New-ScheduledTaskTrigger -AtStartup
$s = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask aIwalkAgent -Action $a -Trigger $t -Settings $s -User SYSTEM -RunLevel Highest -Force | Out-Null
Start-ScheduledTask aIwalkAgent
Write-Host "aIwalk helper is installed. This window closes by itself." -ForegroundColor Green
Start-Sleep 5
