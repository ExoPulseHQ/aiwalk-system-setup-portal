# aIwalk System Setup: Windows VM disk cleanup, run by the aIwalk helper as SYSTEM.
# ASCII only: Windows PowerShell 5.1 reads .ps1 as ANSI/CP950, non-ASCII breaks parsing.
$ErrorActionPreference = 'SilentlyContinue'

function Show-Disk($label) {
    $d = Get-PSDrive C
    "{0,-8} Used {1,6:N1} GB   Free {2,6:N1} GB" -f $label, ($d.Used/1GB), ($d.Free/1GB)
}

Show-Disk "BEFORE"
# running as SYSTEM: clean every user's folders, not SYSTEM's own
# each step also goes to the shared folder so the app can show a progress bar
function Step($text) { Set-Content '\\host.lan\Data\.aiwalk\progress' $text; $text }
$profiles = Get-ChildItem C:\Users -Directory | ForEach-Object { $_.FullName }

Step "1/9 kernel crash dumps"
Remove-Item 'C:\Windows\LiveKernelReports\*.dmp' -Force
Remove-Item 'C:\Windows\LiveKernelReports\*\*' -Recurse -Force
Remove-Item 'C:\Windows\Minidump\*' -Recurse -Force
Remove-Item 'C:\Windows\MEMORY.DMP' -Force

Step "2/9 PowerPoint autorecover cache"
foreach ($p in $profiles) { Remove-Item "$p\AppData\Roaming\Microsoft\PowerPoint\*" -Recurse -Force }

Step "3/9 Office document cache"
foreach ($p in $profiles) { Remove-Item "$p\AppData\Local\Microsoft\Office\16.0\OfficeFileCache\*" -Recurse -Force }

Step "4/9 LINE cache"
foreach ($p in $profiles) { Remove-Item "$p\AppData\Local\LINE\Cache\*" -Recurse -Force }

Step "5/9 temp + windows update download cache"
foreach ($p in $profiles) { Remove-Item "$p\AppData\Local\Temp\*" -Recurse -Force }
Remove-Item 'C:\Windows\Temp\*' -Recurse -Force -Exclude 'aiwalk*'
Remove-Item 'C:\Windows\SoftwareDistribution\Download\*' -Recurse -Force

Step "6/9 recycle bin"
Remove-Item 'C:\$Recycle.Bin\*\*' -Recurse -Force

Step "7/9 disable hibernation, pin pagefile to 1 GB"
powercfg /h off
$cs = Get-WmiObject Win32_ComputerSystem -EnableAllPrivileges
$cs.AutomaticManagedPagefile = $false
$cs.Put() | Out-Null
$pf = Get-WmiObject Win32_PageFileSetting
if ($pf) { $pf.InitialSize = 1024; $pf.MaximumSize = 1024; $pf.Put() | Out-Null }

Step "8/9 WinSxS component cleanup (slow, 5-10 min)"
Dism /Online /Cleanup-Image /StartComponentCleanup /ResetBase | Out-Null

Step "9/9 ReTrim - this is what shrinks the host image file"
Optimize-Volume -DriveLetter C -ReTrim

Show-Disk "AFTER"
""
"Largest files in Downloads (delete these yourself if you do not need them):"
foreach ($p in $profiles) {
    Get-ChildItem "$p\Downloads" -Force -File |
        Sort-Object Length -Descending |
        Select-Object -First 5 @{n='MB'; e={ [math]::Round($_.Length/1MB) }}, FullName |
        Format-Table -AutoSize -HideTableHeaders | Out-String
}
